//! Remote images in messages (W10, ATT-1), fetched only when the user asks:
//! a plain HTTPS GET with no cookies, no referer and no credentials, image
//! types only, at most 10 MB each, cached in the store's `images` table.

use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use gpui::{Context, Task};

use super::{
    Backend,
    http::{Request, Response, Transport, TransportError, log_line},
    now_ms,
    store::{IMAGE_CACHE_CAP, Store},
};
use crate::html::is_safe_image_url;

/// The most one image may weigh.
pub const IMAGE_LIMIT: usize = 10 * 1024 * 1024;
/// Fetches at once.
const IN_FLIGHT: usize = 4;

/// An image ready to show: its type, bytes and size in pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FetchedImage {
    pub mime: &'static str,
    pub bytes: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// The network for images: no cookies (ureq keeps none), no referer, HTTPS
/// only, redirects included, a short timeout and the size cap.
pub struct ImageTransport {
    agent: ureq::Agent,
}

impl Default for ImageTransport {
    fn default() -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .https_only(true)
            .max_redirects(5)
            .timeout_connect(Some(Duration::from_secs(10)))
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("Mozilla/5.0")
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Transport for ImageTransport {
    fn send(&self, request: &Request) -> Result<Response, TransportError> {
        let started = Instant::now();
        let run = || -> Result<Response, ureq::Error> {
            let mut builder = ureq::http::Request::builder().uri(&request.url);
            for (name, value) in &request.headers {
                builder = builder.header(name, value);
            }
            let response = self.agent.run(builder.body(())?)?;
            let status = response.status().as_u16();
            let headers = response
                .headers()
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_str().unwrap_or_default().to_string()))
                .collect();
            let body = response
                .into_body()
                .with_config()
                .limit(IMAGE_LIMIT as u64)
                .read_to_vec()?;
            Ok(Response {
                status,
                headers,
                body,
            })
        };
        let result = run();
        let status = result.as_ref().ok().map(|r| r.status);
        eprintln!(
            "Pigeon: {}",
            log_line(request.method, &request.url, status, started.elapsed())
        );
        result.map_err(|e| TransportError::Network(e.to_string()))
    }
}

/// The image at `url`, from the cache or fetched (then cached). Blocking.
pub fn fetch_image(
    transport: &dyn Transport,
    store: &Store,
    url: &str,
) -> anyhow::Result<FetchedImage> {
    if !is_safe_image_url(url) {
        bail!("not an address images load from");
    }
    if let Some((_, bytes)) = store.image(url)? {
        return decode(bytes);
    }
    let fetch_url = match url.get(..7) {
        Some(scheme) if scheme.eq_ignore_ascii_case("http://") => format!("https://{}", &url[7..]),
        _ => url.to_string(),
    };
    let mut request = Request::get(fetch_url, 0);
    request.headers.push((
        "Accept".into(),
        "image/png,image/jpeg,image/gif,image/webp".into(),
    ));
    let response = transport.send(&request)?;
    if !response.is_success() {
        bail!("HTTP {}", response.status);
    }
    let content_type = response
        .headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_default();
    if !content_type.starts_with("image/") {
        bail!("not an image ({content_type})");
    }
    if response.body.len() > IMAGE_LIMIT {
        bail!("too large");
    }
    let image = decode(response.body)?;
    store.put_image(url, image.mime, &image.bytes, now_ms(), IMAGE_CACHE_CAP)?;
    Ok(image)
}

fn decode(bytes: Vec<u8>) -> anyhow::Result<FetchedImage> {
    let (mime, width, height) = sniff(&bytes).ok_or_else(|| anyhow!("unreadable image"))?;
    if width == 0 || height == 0 {
        bail!("empty image");
    }
    Ok(FetchedImage {
        mime,
        bytes,
        width,
        height,
    })
}

/// The type and pixel size from an image's header: PNG, JPEG, GIF or WebP.
/// Anything else (SVG included) isn't shown.
pub fn sniff(b: &[u8]) -> Option<(&'static str, u32, u32)> {
    let be16 = |i: usize| Some(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let le16 = |i: usize| Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32);
    let be32 = |i: usize| Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?));
    let le24 = |i: usize| {
        let s = b.get(i..i + 3)?;
        Some(s[0] as u32 | (s[1] as u32) << 8 | (s[2] as u32) << 16)
    };
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(("image/png", be32(16)?, be32(20)?));
    }
    if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        return Some(("image/gif", le16(6)?, le16(8)?));
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return match b.get(12..16)? {
            b"VP8 " => Some(("image/webp", le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            b"VP8L" => {
                let bits = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some((
                    "image/webp",
                    (bits & 0x3fff) + 1,
                    ((bits >> 14) & 0x3fff) + 1,
                ))
            }
            b"VP8X" => Some(("image/webp", le24(24)? + 1, le24(27)? + 1)),
            _ => None,
        };
    }
    if b.starts_with(&[0xff, 0xd8]) {
        // Walk the segments to a start-of-frame.
        let mut i = 2;
        while i + 4 <= b.len() {
            if b[i] != 0xff {
                return None;
            }
            let marker = b[i + 1];
            if marker == 0xff {
                i += 1;
                continue;
            }
            let len = be16(i + 2)? as usize;
            let sof = matches!(marker, 0xc0..=0xcf) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
            if sof {
                return Some(("image/jpeg", be16(i + 7)?, be16(i + 5)?));
            }
            i += 2 + len;
        }
    }
    None
}

impl Backend {
    /// Fetch a message's remote images off the main thread, at most four
    /// at a time; each comes back with what was found, `None` when it
    /// couldn't load. Never in demo mode, which has no network.
    pub fn fetch_images(
        &self,
        urls: Vec<String>,
        cx: &mut Context<Self>,
    ) -> Task<Vec<(String, Option<FetchedImage>)>> {
        if self.is_demo() || self.transport().is_none() {
            return Task::ready(urls.into_iter().map(|u| (u, None)).collect());
        }
        let store = self.store.clone();
        let executor = cx.background_executor().clone();
        cx.background_executor().spawn(async move {
            let transport = std::sync::Arc::new(ImageTransport::default());
            let chunk = urls.len().div_ceil(IN_FLIGHT).max(1);
            let tasks: Vec<_> = urls
                .chunks(chunk)
                .map(|urls| {
                    let (urls, store, transport) =
                        (urls.to_vec(), store.clone(), transport.clone());
                    executor.spawn(async move {
                        urls.into_iter()
                            .map(|url| {
                                let image = fetch_image(&*transport, &store, &url)
                                    .map_err(|e| eprintln!("Pigeon: an image didn't load: {e:#}"))
                                    .ok();
                                (url, image)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            let mut out = Vec::new();
            for task in tasks {
                out.extend(task.await);
            }
            out
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::http::FakeTransport;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend(width.to_be_bytes());
        b.extend(height.to_be_bytes());
        b.extend([8, 6, 0, 0, 0]);
        b
    }

    #[test]
    fn sniffs_types_and_sizes() {
        assert_eq!(sniff(&png(600, 300)), Some(("image/png", 600, 300)));
        let mut gif = b"GIF89a".to_vec();
        gif.extend([0x2c, 0x01, 0x64, 0x00]);
        assert_eq!(sniff(&gif), Some(("image/gif", 300, 100)));
        let jpeg = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, // APP0
            0xff, 0xc0, 0x00, 0x11, 0x08, 0x00, 0x20, 0x00, 0x40, // SOF0 64x32
        ];
        assert_eq!(sniff(&jpeg), Some(("image/jpeg", 64, 32)));
        assert_eq!(sniff(b"<svg xmlns='http://www.w3.org/2000/svg'/>"), None);
    }

    #[test]
    fn fetches_once_then_from_the_cache() {
        let store = Store::open_in_memory().unwrap();
        let fake = FakeTransport::new().on("GET", "/a.png", &[], &[(404, "")]);
        assert!(fetch_image(&fake, &store, "https://cdn.example.com/a.png").is_err());
        assert!(
            fetch_image(&fake, &store, "https://127.0.0.1/a.png").is_err(),
            "unsafe addresses never go out"
        );
        assert_eq!(fake.requests().len(), 1);
        assert!(fake.requests()[0].url.starts_with("https://"));

        // Cached already: no request.
        store
            .put_image(
                "http://cdn.example.com/b.png",
                "image/png",
                &png(4, 3),
                1,
                IMAGE_CACHE_CAP,
            )
            .unwrap();
        let image = fetch_image(&fake, &store, "http://cdn.example.com/b.png").unwrap();
        assert_eq!((image.width, image.height), (4, 3));
        assert_eq!(fake.requests().len(), 1);
    }

    #[test]
    fn image_log_lines_hide_the_query() {
        let line = log_line(
            "GET",
            "https://cdn.example.com/open.gif?u=me@example.com&t=abc#x",
            Some(200),
            Duration::from_millis(42),
        );
        assert_eq!(line, "GET cdn.example.com/open.gif?… 200 42ms");
        assert!(!line.contains("me@example.com"));
    }

    #[test]
    fn only_images_are_kept() {
        let store = Store::open_in_memory().unwrap();
        let fake = FakeTransport::new().on("GET", "/page", &[], &[(200, "<html></html>")]);
        // FakeTransport answers without a Content-Type: not an image.
        assert!(fetch_image(&fake, &store, "http://example.com/page").is_err());
        // Upgraded to HTTPS.
        assert_eq!(fake.requests()[0].url, "https://example.com/page");
        assert!(store.image("http://example.com/page").unwrap().is_none());
    }
}
