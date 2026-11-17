"""Isolated, user-led Google sign-in bridge for the native Classroom reader.

The caller receives a short-lived Classroom bearer over a private pipe. No
credential goes into an argument, log, storage-state export, or cache.
"""
import json
import logging
import re
import secrets
import sys
from pathlib import Path

# -I excludes ambient import paths. Load only the bundled companion module.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import session_keychain
import time
from urllib.parse import urlencode, urlsplit

import gpsoauth


GOOGLE_SIGN_IN = "https://accounts.google.com/EmbeddedSetup"
APP_ID = "com.google.android.apps.classroom"
APP_SIGNER = "46f6c8987311e131f4f558d8e0ae145bebab6da3"
SCOPES = (
    "https://www.googleapis.com/auth/homeroom",
    "https://www.googleapis.com/auth/drive",
    "https://www.googleapis.com/auth/plus.peopleapi.readwrite",
)
SAFE_ERRORS = {
    "BadAuthentication", "NeedsBrowser", "ServiceDisabled", "InvalidScope",
    "CaptchaRequired", "DeviceManagementRequired", "InvalidSecondFactor",
    "AccountDisabled", "AppNotAuthorized", "Unauthorized", "TokenExpired",
}


class LoginError(Exception):
    def __init__(self, stage, reason):
        self.stage = stage
        self.reason = reason


def capture_sign_in(email):
    from playwright.sync_api import sync_playwright

    url = GOOGLE_SIGN_IN + "?" + urlencode({"login_hint": email})
    with sync_playwright() as playwright:
        with playwright.chromium.launch(headless=False) as browser:
            with browser.new_context() as context:
                page = context.new_page()
                page.goto(url, wait_until="commit", timeout=30_000)
                page.bring_to_front()
                deadline = time.monotonic() + 300
                while time.monotonic() < deadline:
                    for cookie in context.cookies():
                        if (
                            cookie["name"] == "oauth_token" and cookie["value"]
                            and cookie["domain"].lstrip(".") in ("google.com", "accounts.google.com")
                        ):
                            return cookie["value"]
                    if page.is_closed():
                        raise LoginError("browser", "closed")
                    if urlsplit(page.url).hostname == "accounts.google.com":
                        blocked = page.get_by_text("This browser or app may not be secure", exact=False)
                        if blocked.is_visible():
                            raise LoginError("browser", "google-denied")
                    time.sleep(0.75)
    raise LoginError("browser", "timed-out")


def require_token(reply, field, stage):
    value = reply.get(field)
    if not isinstance(value, str) or not value:
        error = reply.get("Error")
        raise LoginError(stage, error if isinstance(error, str) and error in SAFE_ERRORS else "rejected")
    return value


def exchange(email, mode):
    persistent = mode != "ephemeral"
    remembered = session_keychain.load(email) if mode == "saved-only" else None
    if mode == "saved-only" and remembered is None:
        raise LoginError("saved-session", "missing")
    if remembered:
        master = remembered["master"]
        android_id = remembered["android_id"]
    else:
        login = capture_sign_in(email)
        android_id = secrets.token_hex(8)
        try:
            bootstrap = gpsoauth.exchange_token(email, login, android_id)
        finally:
            del login
        master = require_token(bootstrap, "Token", "account")
        del bootstrap
    try:
        minted = gpsoauth.perform_oauth(
            email, master, android_id, service="oauth2:" + " ".join(SCOPES),
            app=APP_ID, client_sig=APP_SIGNER,
        )
        bearer = require_token(minted, "Auth", "classroom")
        if persistent and not remembered:
            session_keychain.save(email, {"master": master, "android_id": android_id})
    finally:
        del master
        del remembered
    expires_at = minted.get("Expiry")
    try:
        expires_at = int(expires_at)
    except (TypeError, ValueError):
        expires_at = None
    return {"bearer": bearer, "expires_at": expires_at}


def main():
    logging.disable(logging.CRITICAL)
    if sys.stdin.isatty() or sys.stdout.isatty():
        return 2
    email = sys.stdin.readline(257).strip().lower()
    local, _, domain = email.partition("@")
    domain_label = r"[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?"
    if (len(email) > 254 or not re.fullmatch(r"[a-z0-9._+-]{1,128}", local)
            or len(domain) > 253
            or not re.fullmatch(rf"{domain_label}(?:\.{domain_label})+", domain)):
        print('{"stage":"input","reason":"invalid-email"}', flush=True)
        return 2
    try:
        if sys.argv[1:] == ["--forget"]:
            session_keychain.delete(email)
            print('{"removed":true}', flush=True)
            return 0
        modes = {
            (): "interactive",
            ("--saved-only",): "saved-only",
            ("--ephemeral",): "ephemeral",
        }
        mode = modes.get(tuple(sys.argv[1:]))
        if mode is None:
            return 2
        result = exchange(email, mode)
    except LoginError as error:
        print(json.dumps({"stage": error.stage, "reason": error.reason}), flush=True)
        return 1
    except Exception:
        # Exception details from Google and the browser may contain credentials.
        print('{"stage":"runtime","reason":"sign-in-failed"}', flush=True)
        return 1
    print(json.dumps(result, separators=(",", ":")), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
