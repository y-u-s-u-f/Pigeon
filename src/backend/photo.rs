//! The signed-in user's profile photo (ACC-8). `people/me` lists it among the
//! `photos` (the `userinfo.profile` scope covers it); Google's own
//! placeholder for people without one is marked `default` and skipped, so
//! those accounts keep their initials. It is fetched at 128 px, kept in the
//! data folder and fetched again when it is a day old, on launch.

use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{Context as _, bail};
use serde::Deserialize;

use super::{
    http::{Request, Transport},
    images::{IMAGE_LIMIT, sniff},
};

/// The photo's side in pixels: twice the biggest circle it fills.
const SIZE: u32 = 128;
/// How long a cached photo is trusted.
const MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// One entry of a `people` response's `photos`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Photo {
    #[serde(default)]
    url: String,
    /// Google's placeholder silhouette.
    #[serde(default)]
    default: bool,
    #[serde(default)]
    metadata: Metadata,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
struct Metadata {
    #[serde(default)]
    primary: bool,
}

/// The address of the user's own photo, at `SIZE`; none if they have only
/// the placeholder. The primary one wins.
pub fn pick(photos: &[Photo]) -> Option<String> {
    let usable = |p: &&Photo| !p.default && p.url.starts_with("https://");
    photos
        .iter()
        .filter(usable)
        .find(|p| p.metadata.primary)
        .or_else(|| photos.iter().find(usable))
        .map(|p| sized(&p.url))
}

/// `url` with Google's size option set: `…/photo=s100` becomes `…/photo=s128-c`
/// (square crop). Without an option it is added.
fn sized(url: &str) -> String {
    let base = match url.rfind('=') {
        Some(ix) if !url[ix..].contains('/') => &url[..ix],
        _ => url,
    };
    format!("{base}=s{SIZE}-c")
}

/// Where the photo of `email` is kept.
pub fn path(data_dir: &Path, email: &str) -> PathBuf {
    let name: String = email
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    data_dir.join("profile").join(format!("{name}.img"))
}

/// The cached photo, if there is one.
pub fn cached(data_dir: &Path, email: &str) -> Option<PathBuf> {
    let path = path(data_dir, email);
    path.is_file().then_some(path)
}

/// Whether the file at `path` is young enough to keep.
pub fn is_fresh(path: &Path, now: SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age < MAX_AGE)
}

/// Fetch `url` to `path`. Blocking. The file is only replaced by a real
/// image.
pub fn download(transport: &dyn Transport, url: &str, path: &Path) -> anyhow::Result<()> {
    let mut request = Request::get(url, 0);
    request.headers.push((
        "Accept".into(),
        "image/jpeg,image/png,image/gif,image/webp".into(),
    ));
    let response = transport.send(&request)?;
    if !response.is_success() {
        bail!("HTTP {}", response.status);
    }
    if response.body.len() > IMAGE_LIMIT || sniff(&response.body).is_none() {
        bail!("not a usable image");
    }
    let dir = path.parent().context("no folder")?;
    std::fs::create_dir_all(dir)?;
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, &response.body)?;
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// Remove the photo of `email`.
pub fn forget(data_dir: &Path, email: &str) {
    std::fs::remove_file(path(data_dir, email)).ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo(url: &str, default: bool, primary: bool) -> Photo {
        Photo {
            url: url.into(),
            default,
            metadata: Metadata { primary },
        }
    }

    #[test]
    fn people_me_photos_parse() {
        let json = r#"[{"metadata":{"primary":true,"source":{"type":"PROFILE"}},
            "url":"https://lh3.googleusercontent.com/a/abc=s100","default":true},
            {"url":"https://lh3.googleusercontent.com/a/def=s100"}]"#;
        let photos: Vec<Photo> = serde_json::from_str(json).unwrap();
        assert_eq!(photos.len(), 2);
        assert!(photos[0].default && photos[0].metadata.primary);
        assert!(!photos[1].default);
    }

    #[test]
    fn size_option_is_replaced_or_added() {
        assert_eq!(
            sized("https://lh3.googleusercontent.com/a/abc=s100"),
            "https://lh3.googleusercontent.com/a/abc=s128-c"
        );
        assert_eq!(
            sized("https://lh3.googleusercontent.com/a/abc=s64-c-mo"),
            "https://lh3.googleusercontent.com/a/abc=s128-c"
        );
        assert_eq!(
            sized("https://lh3.googleusercontent.com/a/abc"),
            "https://lh3.googleusercontent.com/a/abc=s128-c"
        );
        // An `=` in the path isn't an option.
        assert_eq!(
            sized("https://example.com/a=b/photo"),
            "https://example.com/a=b/photo=s128-c"
        );
    }

    #[test]
    fn the_placeholder_is_skipped() {
        let only_default = [photo(
            "https://lh3.googleusercontent.com/a/x=s100",
            true,
            true,
        )];
        assert_eq!(pick(&only_default), None);
        assert_eq!(pick(&[]), None);
    }

    #[test]
    fn the_primary_real_photo_wins() {
        let photos = [
            photo("https://lh3.googleusercontent.com/a/one=s100", false, false),
            photo("https://lh3.googleusercontent.com/a/two=s100", false, true),
            photo(
                "https://lh3.googleusercontent.com/a/three=s100",
                true,
                false,
            ),
        ];
        assert_eq!(
            pick(&photos).as_deref(),
            Some("https://lh3.googleusercontent.com/a/two=s128-c")
        );
        // A real photo that isn't primary beats a primary placeholder.
        let photos = [
            photo("https://lh3.googleusercontent.com/a/one=s100", true, true),
            photo("https://lh3.googleusercontent.com/a/two=s100", false, false),
        ];
        assert_eq!(
            pick(&photos).as_deref(),
            Some("https://lh3.googleusercontent.com/a/two=s128-c")
        );
    }

    #[test]
    fn only_https_photos_are_used() {
        assert_eq!(pick(&[photo("http://example.com/p", false, true)]), None);
    }

    #[test]
    fn the_path_is_safe_per_account() {
        let dir = Path::new("/data");
        assert_eq!(
            path(dir, "alex.r@example.com"),
            Path::new("/data/profile/alex_r_example_com.img")
        );
        assert_eq!(path(dir, "../x@y"), Path::new("/data/profile/___x_y.img"));
    }

    #[test]
    fn a_day_old_file_is_stale() {
        let dir = std::env::temp_dir().join(format!("pigeon-photo-{}", uuid::Uuid::new_v4()));
        let file = path(&dir, "a@b.c");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"x").unwrap();
        assert!(is_fresh(&file, SystemTime::now()));
        assert!(!is_fresh(
            &file,
            SystemTime::now() + MAX_AGE + Duration::from_secs(60)
        ));
        assert_eq!(cached(&dir, "a@b.c"), Some(file));
        forget(&dir, "a@b.c");
        assert_eq!(cached(&dir, "a@b.c"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_file_is_not_fresh() {
        assert!(!is_fresh(
            Path::new("/nonexistent/photo.img"),
            SystemTime::now()
        ));
    }
}
