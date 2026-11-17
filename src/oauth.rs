use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{
    digest::{SHA256, digest},
    rand::{SecureRandom, SystemRandom},
};
use std::{
    error::Error,
    io,
    process::Command,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    time::timeout,
};

const REDIRECT_PATH: &str = "/oauth/callback";
const GOOGLE_AUTHORIZATION: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const HOMEROOM_SCOPE: &str = "https://www.googleapis.com/auth/homeroom";

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    CodeReceived,
    Denied(&'static str),
}

pub async fn scope_check(client_id: &str) -> Result<Outcome, Box<dyn Error>> {
    let valid = client_id.len() <= 256
        && client_id
            .strip_suffix(".apps.googleusercontent.com")
            .is_some_and(|id| {
                !id.is_empty() && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            });
    if !valid {
        return Err("Supply the public client ID from your own Google Cloud Desktop app.".into());
    }

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://{}{REDIRECT_PATH}", listener.local_addr()?);
    let state = random_value()?;
    let verifier = random_value()?;
    let challenge = URL_SAFE_NO_PAD.encode(digest(&SHA256, verifier.as_bytes()).as_ref());
    let url = authorization_url(client_id, &redirect, &state, &challenge);
    println!("Opening Google sign-in in your default browser for your registered desktop client.");
    println!("If Google shows an error without returning here, record only the error label and press Ctrl+C.");
    if !Command::new("open").arg(url).status()?.success() {
        return Err("Could not open your default browser.".into());
    }

    let result = timeout(Duration::from_secs(180), async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            if let Ok(Ok(Some(outcome))) = timeout(
                Duration::from_secs(3),
                read_callback(&mut stream, &state),
            )
            .await
            {
                let _ = timeout(Duration::from_secs(1), acknowledge(&mut stream)).await;
                return Ok::<Outcome, io::Error>(outcome);
            }
        }
    })
    .await;
    Ok(result.map_err(|_| "No validated authorization callback arrived within 180 seconds.")??)
}

fn random_value() -> io::Result<String> {
    let mut bytes = [0; 32];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| io::Error::other("Could not generate OAuth randomness."))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

fn authorization_url(client_id: &str, redirect: &str, state: &str, challenge: &str) -> String {
    let query = form_urlencoded::Serializer::new(String::new())
        .extend_pairs([
            ("client_id", client_id),
            ("redirect_uri", redirect),
            ("response_type", "code"),
            ("scope", HOMEROOM_SCOPE),
            ("state", state),
            ("code_challenge", challenge),
            ("code_challenge_method", "S256"),
            ("prompt", "select_account"),
        ])
        .finish();
    format!("{GOOGLE_AUTHORIZATION}?{query}")
}

async fn read_callback(stream: &mut TcpStream, state: &str) -> io::Result<Option<Outcome>> {
    let mut bytes = Vec::new();
    stream.take(8192).read_buf(&mut bytes).await?;
    let mut reader = stream.take(8192_u64.saturating_sub(bytes.len() as u64));
    while !bytes.contains(&b'\n') {
        if reader.read_buf(&mut bytes).await? == 0 {
            return Ok(None);
        }
    }
    let line = bytes.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let outcome = std::str::from_utf8(line)
        .ok()
        .and_then(|line| parse_callback(line, state));
    Ok(outcome)
}

fn parse_callback(line: &str, expected_state: &str) -> Option<Outcome> {
    let mut fields = line.split_ascii_whitespace();
    if fields.next()? != "GET" {
        return None;
    }
    let (path, query) = fields.next()?.split_once('?')?;
    if path != REDIRECT_PATH || !matches!(fields.next()?, "HTTP/1.0" | "HTTP/1.1") {
        return None;
    }

    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (key, value) in form_urlencoded::parse(query.as_bytes()) {
        let slot = match key.as_ref() {
            "state" => &mut state,
            "code" => &mut code,
            "error" => &mut error,
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return None;
        }
    }
    if state.as_deref() != Some(expected_state) {
        return None;
    }
    match (code, error) {
        (Some(value), None) if !value.is_empty() => Some(Outcome::CodeReceived),
        (None, Some(value)) => Some(Outcome::Denied(match value.as_ref() {
            "access_denied" => "access_denied",
            "admin_policy_enforced" => "admin_policy_enforced",
            "invalid_scope" => "invalid_scope",
            "unauthorized_client" => "unauthorized_client",
            "org_internal" => "org_internal",
            "invalid_request" => "invalid_request",
            _ => "other authorization error",
        })),
        _ => None,
    }
}

async fn acknowledge(stream: &mut TcpStream) -> io::Result<()> {
    let body = "Scope check finished. Return to the terminal for the result.";
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(reply.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_requires_expected_state_and_never_returns_code() {
        assert_eq!(
            parse_callback("GET /oauth/callback?state=match&code=example HTTP/1.1\r", "match"),
            Some(Outcome::CodeReceived)
        );
        assert_eq!(
            parse_callback("GET /oauth/callback?state=wrong&code=example HTTP/1.1", "match"),
            None
        );
        assert_eq!(
            parse_callback("GET /oauth/callback?state=match&error=invalid_scope HTTP/1.1", "match"),
            Some(Outcome::Denied("invalid_scope"))
        );
        assert_eq!(
            parse_callback("GET /oauth/callback?state=match&state=match&code=example HTTP/1.1", "match"),
            None
        );
    }

    #[test]
    fn authorization_uses_google_desktop_loopback_with_pkce() {
        let url = authorization_url(
            "own-client.apps.googleusercontent.com",
            "http://127.0.0.1:1234/oauth/callback",
            "state",
            "challenge",
        );
        assert!(url.starts_with(GOOGLE_AUTHORIZATION));
        let (_, query) = url.split_once('?').unwrap();
        let values: std::collections::HashMap<_, _> =
            form_urlencoded::parse(query.as_bytes()).collect();
        assert_eq!(values["scope"], HOMEROOM_SCOPE);
        assert_eq!(values["redirect_uri"], "http://127.0.0.1:1234/oauth/callback");
        assert_eq!(values["code_challenge_method"], "S256");
    }
}
