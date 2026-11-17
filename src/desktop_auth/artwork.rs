//! Load only the artwork URLs returned by the course response, with no Google
//! tokens, cookies, or on-disk image cache. Redirects are checked before each hop.

use iced::{
    futures::{Stream, StreamExt, future, stream},
    widget::image::Handle,
};
use std::{collections::BTreeSet, io::Cursor, process::Stdio, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, process::Command, time::timeout};

const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CONCURRENT: usize = 4;
const MAX_ARTWORKS: usize = 64;
const MAX_REDIRECTS: usize = 3;
const MAX_RESPONSE_METADATA: u64 = 8192 + 5;

pub async fn load_artworks(urls: Vec<String>) -> Vec<(String, Handle)> {
    artwork_updates(urls).collect().await
}

pub fn artwork_updates(urls: Vec<String>) -> impl Stream<Item = (String, Handle)> + Send + 'static {
    let mut seen = BTreeSet::new();
    let urls: Vec<_> = urls.into_iter()
        .filter(|url| valid_photo_url(url) && seen.insert(url.clone()))
        .take(MAX_ARTWORKS)
        .collect();
    stream::iter(urls)
        .map(fetch)
        .buffer_unordered(MAX_CONCURRENT)
        .filter_map(future::ready)
}

async fn fetch(url: String) -> Option<(String, Handle)> {
    let bytes = timeout(Duration::from_secs(14), fetch_bytes(&url)).await.ok()??;
    let size = bytes.len();
    let image = tokio::task::spawn_blocking(move || decode(bytes)).await.ok()?;
    if image.is_none() && diagnostics_enabled() {
        eprintln!("Artwork decode failed: bytes={size}");
    }
    Some((url, image?))
}

async fn fetch_bytes(url: &str) -> Option<Vec<u8>> {
    let mut current = url.to_owned();
    for redirects in 0..=MAX_REDIRECTS {
        let response = request_photo(&current).await?;
        if response.status == 200 && !response.body.is_empty() {
            return Some(response.body);
        }
        current = checked_redirect(response.status, &response.redirect, redirects)?;
    }
    None
}

fn checked_redirect(status: u16, target: &str, redirects: usize) -> Option<String> {
    (redirects < MAX_REDIRECTS && matches!(status, 301 | 302 | 303 | 307 | 308)
        && valid_photo_url(target)).then(|| target.to_owned())
}

fn valid_photo_url(url: &str) -> bool {
    super::artwork_host_is_allowed(url) && !url.contains('"')
}

struct PhotoResponse {
    status: u16,
    redirect: String,
    body: Vec<u8>,
}

async fn request_photo(url: &str) -> Option<PhotoResponse> {
    if !valid_photo_url(url) {
        return None;
    }
    let mut child = Command::new("/usr/bin/curl")
        // --disable must come first to ignore the user's curl configuration.
        .args(["--disable", "--fail", "--silent", "--globoff", "--max-time", "12",
            "--max-filesize", "4194304", "--proto", "=https", "--config", "-",
            "--write-out", "%{stderr}%{http_code}\n%{redirect_url}\n"])
        .env_remove("SSLKEYLOGFILE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn().ok()?;

    let mut config = child.stdin.take()?;
    // Both the initial URL and redirects may contain share keys.
    config.write_all(format!("url = \"{url}\"\n").as_bytes()).await.ok()?;
    drop(config);
    let mut body_pipe = child.stdout.take()?.take(MAX_IMAGE_BYTES + 1);
    let mut meta_pipe = child.stderr.take()?.take(MAX_RESPONSE_METADATA + 1);
    let mut body = Vec::new();
    let mut metadata = Vec::new();
    tokio::try_join!(body_pipe.read_to_end(&mut body), meta_pipe.read_to_end(&mut metadata)).ok()?;
    if body.len() as u64 > MAX_IMAGE_BYTES || metadata.len() as u64 > MAX_RESPONSE_METADATA {
        return None;
    }
    let exit = child.wait().await.ok()?;
    let metadata = std::str::from_utf8(&metadata).ok()?;
    let (status, redirect) = metadata.strip_suffix('\n')?.split_once('\n')?;
    let status = status.parse().ok()?;
    if diagnostics_enabled() {
        eprintln!("Artwork HTTP={status} curl={:?} bytes={}", exit.code(), body.len());
    }
    exit.success().then(|| PhotoResponse { status, redirect: redirect.to_owned(), body })
}

fn diagnostics_enabled() -> bool {
    std::env::var_os("CLASSROOM_READ_ONLY_DIAGNOSTICS").is_some()
}

fn decode(bytes: Vec<u8>) -> Option<Handle> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format().ok()?;
    if !matches!(reader.format(), Some(image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::WebP)) {
        return None;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(80 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(1040, 416).to_rgba8();
    Some(Handle::from_rgba(image.width(), image.height(), image.into_raw()))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_only_bounded_redirects_within_google_image_hosts() {
        let photo = "https://lh3.googleusercontent.com/class-banner";
        assert_eq!(checked_redirect(301, photo, 0).as_deref(), Some(photo));
        assert!(checked_redirect(200, photo, 0).is_none());
        assert!(checked_redirect(302, photo, MAX_REDIRECTS).is_none());
        for unsafe_target in [
            "http://lh3.googleusercontent.com/class-banner",
            "https://googleusercontent.com.example.com/image",
            "https://other.example.com/image",
            "https://lh3.googleusercontent.com@other.example.com/image",
            "https://lh3.googleusercontent.com:8081/class-banner",
            "https://lh3.googleusercontent.com/not\"a\nconfig",
        ] {
            assert!(checked_redirect(302, unsafe_target, 0).is_none());
        }
    }
}
