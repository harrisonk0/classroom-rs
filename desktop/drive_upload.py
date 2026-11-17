"""Read Drive metadata and upload explicitly chosen files."""

import json
import mimetypes
import os
from pathlib import Path
import re
import stat
import sys
from urllib.parse import urlsplit

import requests

UPLOAD_ENDPOINT = "https://www.googleapis.com/upload/drive/v3/files"
FILES_ENDPOINT = "https://www.googleapis.com/drive/v3/files"
FILE_ID = re.compile(r"[A-Za-z0-9_-]{1,256}\Z")


class UploadError(Exception):
    pass


def valid_session_url(url):
    parts = urlsplit(url)
    return (
        parts.scheme == "https"
        and parts.netloc in ("www.googleapis.com", "www.googleapis.com:443")
        and parts.path == "/upload/drive/v3/files"
        and bool(parts.query)
        and not parts.fragment
    )


def upload(path, bearer):
    name = path.name
    if path.suffix.lower() == ".m4a":
        mime_type = "audio/mp4"
    else:
        mime_type = mimetypes.guess_type(name)[0] or "application/octet-stream"
    if not path.is_file():
        raise UploadError("Choose a regular file.")
    with path.open("rb") as source, requests.Session() as session:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_size <= 0:
            raise UploadError("Choose a nonempty regular file.")
        length = info.st_size
        session.trust_env = False
        session.headers["Authorization"] = f"Bearer {bearer}"
        response = session.post(
            UPLOAD_ENDPOINT,
            params={"uploadType": "resumable", "fields": "id,name,mimeType"},
            json={"name": name, "mimeType": mime_type},
            headers={"X-Upload-Content-Type": mime_type, "X-Upload-Content-Length": str(length)},
            allow_redirects=False,
            timeout=(15, 30),
        )
        location = response.headers.get("Location", "")
        if response.status_code != 200 or not valid_session_url(location):
            raise UploadError(f"Drive did not start the upload (HTTP {response.status_code}).")
        completed = session.put(
            location,
            data=source,
            headers={
                "Content-Type": mime_type,
                "Content-Length": str(length),
                "Content-Range": f"bytes 0-{length - 1}/{length}",
            },
            allow_redirects=False,
            timeout=(15, 90),
        )
        if completed.status_code not in (200, 201):
            raise UploadError(f"Drive did not confirm the upload (HTTP {completed.status_code}).")
        file_id = completed.json().get("id")
        if not isinstance(file_id, str) or not FILE_ID.fullmatch(file_id):
            raise UploadError("Drive did not return a file ID. Check Drive before trying again.")
        return {"id": file_id, "name": name, "mime_type": mime_type}


def list_files(bearer, search, page_token, parent=None, shared_only=False):
    query = "trashed = false"
    if parent:
        if not FILE_ID.fullmatch(parent):
            raise UploadError("Invalid Drive folder ID.")
        query += f" and '{parent}' in parents"
    elif shared_only:
        query += " and sharedWithMe"
    else:
        query += " and mimeType != 'application/vnd.google-apps.folder'"
    if search:
        escaped = search.replace("\\", "\\\\").replace("'", "\\'")
        query += f" and name contains '{escaped}'"
    params = {
        "corpora": "user",
        "spaces": "drive",
        "q": query,
        "pageSize": 40,
        "orderBy": "modifiedTime desc",
        "fields": "nextPageToken,incompleteSearch,files(id,name,mimeType)",
        "supportsAllDrives": "true",
        "includeItemsFromAllDrives": "true",
    }
    if page_token:
        params["pageToken"] = page_token
    with requests.Session() as session:
        session.trust_env = False
        session.headers["Authorization"] = f"Bearer {bearer}"
        response = session.get(FILES_ENDPOINT, params=params, allow_redirects=False, timeout=(10, 20))
        if response.status_code != 200:
            raise UploadError(f"Drive could not list files (HTTP {response.status_code}).")
        return response.json()


def get_file_link(bearer, file_id):
    if not isinstance(file_id, str) or not FILE_ID.fullmatch(file_id):
        raise UploadError("Invalid Drive file ID.")
    with requests.Session() as session:
        session.trust_env = False
        session.headers["Authorization"] = f"Bearer {bearer}"
        response = session.get(
            f"{FILES_ENDPOINT}/{file_id}",
            params={"fields": "id,webViewLink,trashed", "supportsAllDrives": "true"},
            allow_redirects=False,
            timeout=(10, 20),
        )
        if response.status_code != 200:
            raise UploadError(f"Drive could not open the file (HTTP {response.status_code}).")
        return response.json()


def download_file(bearer, file_id, temporary_path, resource_key=None, preview=False):
    if not isinstance(file_id, str) or not FILE_ID.fullmatch(file_id):
        raise UploadError("Invalid Drive file ID.")
    if resource_key is not None and (not isinstance(resource_key, str) or not FILE_ID.fullmatch(resource_key)):
        raise UploadError("Invalid Drive resource key.")
    with requests.Session() as session:
        session.trust_env = False
        session.headers["Authorization"] = f"Bearer {bearer}"
        session.headers["Accept-Encoding"] = "identity"
        if resource_key:
            session.headers["X-Goog-Drive-Resource-Keys"] = f"{file_id}/{resource_key}"
        endpoint = f"{FILES_ENDPOINT}/{file_id}"
        metadata = session.get(endpoint, params={
            "fields": "id,mimeType,name,trashed,capabilities(canDownload)",
            "supportsAllDrives": "true",
        }, allow_redirects=False, timeout=(10, 30))
        if metadata.status_code != 200:
            raise UploadError(f"Drive could not read file details (HTTP {metadata.status_code}).")
        details = metadata.json()
        mime = details.get("mimeType")
        if (details.get("id") != file_id or details.get("trashed")
                or not isinstance(mime, str)
                or details.get("capabilities", {}).get("canDownload") is not True):
            raise UploadError("Drive did not allow downloading this file. Open it in Drive instead.")
        name = details.get("name") or "Classroom attachment"
        params = {"alt": "media", "supportsAllDrives": "true"}
        if mime.startswith("application/vnd.google-apps."):
            if not preview or mime not in {
                "application/vnd.google-apps.document", "application/vnd.google-apps.spreadsheet",
                "application/vnd.google-apps.presentation", "application/vnd.google-apps.drawing",
            }:
                raise UploadError("Drive cannot download this type of file here. Open it in Drive instead.")
            endpoint += "/export"
            params = {"mimeType": "application/pdf"}
            name = name if name.lower().endswith(".pdf") else name + ".pdf"
        elif preview and not Path(name).suffix:
            name += mimetypes.guess_extension(mime, strict=False) or ""
        with session.get(endpoint, params=params,
                         allow_redirects=False, timeout=(15, 90), stream=True) as response:
            if response.status_code != 200:
                raise UploadError(f"Drive could not download the file (HTTP {response.status_code}).")
            with open(temporary_path, "wb") as output:
                for chunk in response.iter_content(chunk_size=1024 * 1024):
                    if chunk:
                        output.write(chunk)
                output.flush()
                os.fsync(output.fileno())
            expected = response.headers.get("Content-Length")
            if expected is not None and int(expected) != os.path.getsize(temporary_path):
                raise UploadError("Drive returned an incomplete file.")
    return {"saved": True, "name": name} if preview else {"saved": True}


def main():
    if sys.stdin.isatty() or sys.stdout.isatty():
        return 2
    listing = sys.argv[1:] == ["--list"]
    lookup = sys.argv[1:] == ["--file-link"]
    downloading = sys.argv[1:] == ["--download"]
    try:
        request = json.loads(sys.stdin.readline(65536))
        if listing:
            result = list_files(request["bearer"], request["search"], request["page_token"],
                                request.get("parent"), request.get("shared_only", False))
        elif lookup:
            result = get_file_link(request["bearer"], request["file_id"])
        elif downloading:
            result = download_file(request["bearer"], request["file_id"], request["temporary_path"],
                                   request.get("resource_key"), request.get("preview", False))
        else:
            result = upload(Path(request["path"]), request["bearer"])
    except UploadError as error:
        print(json.dumps({"error": str(error)}), flush=True)
        return 1
    except Exception:
        # Avoid exposing the bearer, path, or upload session URL in exceptions.
        if downloading:
            message = "Drive could not download the file."
        elif lookup:
            message = "Drive could not open the file."
        elif listing:
            message = "Drive could not list files."
        else:
            message = "Drive upload failed. Check Drive before trying again."
        print(json.dumps({"error": message}), flush=True)
        return 1
    print(json.dumps(result), flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
