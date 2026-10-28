//! Reader extras (W10): remote images fetched at the user's request
//! (ATT-1), capped by total size, and invitations by iCalendar UID (CAL-20).

use rusqlite::{OptionalExtension as _, params};
use sha2::{Digest as _, Sha256};

use super::Store;

/// How much the image cache keeps, all images together.
pub const IMAGE_CACHE_CAP: i64 = 64 * 1024 * 1024;

fn url_hash(url: &str) -> String {
    format!("{:x}", Sha256::digest(url.as_bytes()))
}

impl Store {
    /// A cached image: its MIME type and bytes.
    pub fn image(&self, url: &str) -> anyhow::Result<Option<(String, Vec<u8>)>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT mime, bytes FROM images WHERE url_hash = ?1",
                [url_hash(url)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Keep an image, then drop the oldest until the cache fits in `cap`
    /// bytes.
    pub fn put_image(
        &self,
        url: &str,
        mime: &str,
        bytes: &[u8],
        now: i64,
        cap: i64,
    ) -> anyhow::Result<()> {
        self.put_images([(url, mime, bytes)], now, cap)
    }

    /// Keep `images` (url, type, bytes) in one transaction, then drop the
    /// oldest until the cache fits in `cap` bytes, in one statement.
    pub fn put_images<'a>(
        &self,
        images: impl IntoIterator<Item = (&'a str, &'a str, &'a [u8])>,
        now: i64,
        cap: i64,
    ) -> anyhow::Result<()> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        for (url, mime, bytes) in images {
            tx.prepare_cached(
                "INSERT INTO images (url_hash, url, mime, bytes, fetched_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (url_hash) DO UPDATE SET
                    mime = excluded.mime, bytes = excluded.bytes, fetched_at = excluded.fetched_at",
            )?
            .execute(params![url_hash(url), url, mime, bytes, now])?;
        }
        tx.execute(
            "DELETE FROM images WHERE rowid IN (
                SELECT rowid FROM (
                    SELECT rowid, SUM(length(bytes)) OVER (
                        ORDER BY fetched_at DESC, rowid DESC
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
                    ) AS total FROM images
                ) WHERE total > ?1
            )",
            [cap],
        )?;
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_cache_keeps_the_newest_within_its_cap() {
        let store = Store::open_in_memory().unwrap();
        store
            .put_image("https://a/1", "image/png", &[1; 40], 1, 100)
            .unwrap();
        store
            .put_image("https://a/2", "image/png", &[2; 40], 2, 100)
            .unwrap();
        assert!(store.image("https://a/1").unwrap().is_some());
        // A third doesn't fit with both: the oldest goes.
        store
            .put_image("https://a/3", "image/gif", &[3; 40], 3, 100)
            .unwrap();
        assert!(store.image("https://a/1").unwrap().is_none());
        assert_eq!(
            store.image("https://a/3").unwrap(),
            Some(("image/gif".into(), vec![3; 40]))
        );
        assert!(store.image("https://a/2").unwrap().is_some());
        // Fetching again refreshes it.
        store
            .put_image("https://a/2", "image/png", &[2; 40], 4, 100)
            .unwrap();
        store
            .put_image("https://a/4", "image/png", &[4; 40], 5, 100)
            .unwrap();
        assert!(store.image("https://a/3").unwrap().is_none());
        assert!(store.image("https://a/2").unwrap().is_some());
    }
}
