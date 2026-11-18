pub mod database_migrations;
pub mod database_service;
pub mod entities;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::Row;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, Value, ValueRef};

/// GRDB `FetchableRecord & PersistableRecord & TableRecord`: one row of `TABLE`.
pub trait Record: Sized {
    const TABLE: &'static str;
    /// Columns in the order `values()` returns them.
    const COLUMNS: &'static [&'static str];
    /// Every entity table has a single-column primary key.
    const PRIMARY_KEY: &'static str = "id";

    /// Reads a row by column name, so `SELECT *` and `SELECT song.*` both work.
    fn from_row(row: &Row) -> rusqlite::Result<Self>;
    fn values(&self) -> Vec<Value>;
}

/// GRDB's `Date` encoding: UTC text `yyyy-MM-dd HH:mm:ss.SSS`, so text order is time order.
pub fn date_text(date: SystemTime) -> String {
    let ms = match date.duration_since(UNIX_EPOCH) {
        Ok(after) => after.as_millis() as i64,
        Err(before) => -(before.duration().as_millis() as i64),
    };
    let (days, ms_of_day) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}.{:03}",
        ms_of_day / 3_600_000,
        ms_of_day / 60_000 % 60,
        ms_of_day / 1000 % 60,
        ms_of_day % 1000
    )
}

/// A `Date` column or argument. Reads GRDB text (`yyyy-MM-dd[ HH:mm[:ss[.SSS]]]`,
/// `T` separator and trailing `Z` allowed) and numbers as Unix seconds, like GRDB.
// ponytail: no `+HH:MM` offsets; GRDB and Trop only write UTC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DbDate(pub SystemTime);

impl DbDate {
    pub fn parse(text: &str) -> Option<SystemTime> {
        let text = text.strip_suffix('Z').unwrap_or(text);
        let (date, time) = text.split_once([' ', 'T']).unwrap_or((text, "00:00"));
        let date: Vec<i64> = date
            .split('-')
            .map(|p| p.parse().ok())
            .collect::<Option<_>>()?;
        let [year, month, day] = date[..] else {
            return None;
        };
        let mut time = time.split(':');
        let hours: i64 = time.next()?.parse().ok()?;
        let minutes: i64 = time.next()?.parse().ok()?;
        let seconds: f64 = time.next().map_or(Some(0.0), |s| s.parse().ok())?;
        if time.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let ms = (days_from_civil(year, month, day) * 86_400 + hours * 3600 + minutes * 60) * 1000
            + (seconds * 1000.0).round() as i64;
        Some(from_millis(ms))
    }
}

impl FromSql for DbDate {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        match value {
            ValueRef::Integer(secs) => Ok(Self(from_millis(secs * 1000))),
            ValueRef::Real(secs) => Ok(Self(from_millis((secs * 1000.0).round() as i64))),
            ValueRef::Text(_) => Self::parse(value.as_str()?)
                .map(Self)
                .ok_or(FromSqlError::InvalidType),
            _ => Err(FromSqlError::InvalidType),
        }
    }
}

impl ToSql for DbDate {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::Owned(Value::Text(date_text(self.0))))
    }
}

/// Reads a non-null date column.
pub(crate) fn get_date(row: &Row, column: &str) -> rusqlite::Result<SystemTime> {
    Ok(row.get::<_, DbDate>(column)?.0)
}

/// Reads a nullable date column.
pub(crate) fn get_opt_date(row: &Row, column: &str) -> rusqlite::Result<Option<SystemTime>> {
    Ok(row.get::<_, Option<DbDate>>(column)?.map(|d| d.0))
}

/// Date column value for `Record::values`.
pub(crate) fn date_value(date: Option<SystemTime>) -> Value {
    date.map(date_text).into()
}

fn from_millis(ms: i64) -> SystemTime {
    let span = Duration::from_millis(ms.unsigned_abs());
    if ms >= 0 {
        UNIX_EPOCH + span
    } else {
        UNIX_EPOCH - span
    }
}

// Proleptic Gregorian calendar <-> days since 1970-01-01 (Howard Hinnant's algorithms).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}
