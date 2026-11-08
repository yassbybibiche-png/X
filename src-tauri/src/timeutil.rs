//! Dates : fuseau local et conversion vers le format DOS des ZIP.

use std::time::SystemTime;

use time::{OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// Décalage du fuseau local (UTC si indéterminable, ex. processus multi-thread
/// sous Linux où la crate `time` refuse de le lire).
pub fn local_offset() -> UtcOffset {
    UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
}

/// Date ZIP (format DOS, heure locale, précision 2 s, années 1980-2107).
pub fn zip_datetime(t: SystemTime) -> Option<zip::DateTime> {
    let local = OffsetDateTime::from(t).to_offset(local_offset());
    zip::DateTime::try_from(PrimitiveDateTime::new(local.date(), local.time())).ok()
}

/// Secondes Unix (bornées à `u32`) pour le champ ZIP « extended timestamp ».
pub fn unix_seconds(t: SystemTime) -> Option<u32> {
    let secs = t.duration_since(SystemTime::UNIX_EPOCH).ok()?.as_secs();
    u32::try_from(secs).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    #[test]
    fn conversions() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(unix_seconds(t), Some(1_700_000_000));
        let dos = zip_datetime(t).unwrap();
        assert_eq!(dos.year(), 2023);
        assert!(zip_datetime(UNIX_EPOCH).is_none(), "avant 1980");
    }
}
