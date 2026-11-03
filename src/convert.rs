use crate::bindings::supabase::wrappers::types::{Cell, FdwError, TypeOid};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use serde_json::Value;
use std::str::FromStr;

// Parse integers directly into the target range without rounding through f64.
fn integer<T: FromStr>(value: &Value) -> Result<T, String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(number) if number.is_i64() || number.is_u64() => number.to_string(),
        _ => return Err("expected an integer or integer string".into()),
    };
    text.parse()
        .map_err(|_| "integer out of range or invalid integer".into())
}

// Accept only finite numbers; the WIT Numeric carrier is also f64.
fn real(value: &Value) -> Result<f64, String> {
    let text = match value {
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => return Err("expected a finite number or numeric string".into()),
    };
    let number = text
        .parse::<f64>()
        .map_err(|_| "expected a finite number or numeric string")?;
    // Inspect mantissa digits to reject underflow while preserving valid signed zero.
    if number == 0.0
        && text
            .split(['e', 'E'])
            .next()
            .unwrap_or("")
            .chars()
            .any(|c| matches!(c, '1'..='9'))
    {
        return Err("number underflows f64 range".into());
    }
    if number.is_finite() {
        Ok(number)
    } else {
        Err("non-finite number".into())
    }
}

// Normalize RFC3339 offsets to UTC; interpret unzoned SQL timestamps as UTC.
fn timestamp(text: &str) -> Result<i64, String> {
    if let Ok(value) = DateTime::parse_from_rfc3339(text) {
        return Ok(value.timestamp_micros());
    }
    for format in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S%.f"] {
        if let Ok(value) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(value.and_utc().timestamp_micros());
        }
    }
    Err("expected RFC3339 or YYYY-MM-DD HH:MM:SS[.ffffff]".into())
}

// Compare decimal significands and exponents without floating-point rounding.
fn decimal_key(text: &str) -> Result<(bool, String, i64), String> {
    let mut parts = text.split(['e', 'E']);
    let mantissa = parts.next().unwrap_or("");
    let exponent = parts
        .next()
        .unwrap_or("0")
        .parse::<i64>()
        .map_err(|_| "JSON exponent out of range")?;
    let negative = mantissa.starts_with('-');
    let mantissa = mantissa.trim_start_matches('-');
    let fractional = mantissa
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let digits = digits.trim_start_matches('0');
    if digits.is_empty() {
        return Ok((false, "0".into(), 0));
    }
    let trimmed = digits.trim_end_matches('0');
    let exponent = exponent
        .checked_sub(fractional as i64)
        .and_then(|v| v.checked_add((digits.len() - trimmed.len()) as i64))
        .ok_or("JSON exponent out of range")?;
    Ok((negative, trimmed.into(), exponent))
}

// The Wrappers 0.6.1 host reparses JSON; reject values it would silently round.
fn validate_host_json(value: &Value) -> Result<(), String> {
    match value {
        Value::Number(number) if !number.is_i64() && !number.is_u64() => {
            let compatible = number.as_f64().and_then(serde_json::Number::from_f64);
            if compatible.as_ref().map(|n| decimal_key(&n.to_string()))
                != Some(decimal_key(&number.to_string()))
            {
                return Err(
                    "JSON number would lose precision in the Wasm host; map this column as TEXT"
                        .into(),
                );
            }
        }
        Value::Array(values) => {
            for value in values {
                validate_host_json(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                validate_host_json(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// Preserve NULL and reject invalid types, out-of-range values and invalid dates.
pub(crate) fn source_cell(row: &Value, name: &str, oid: TypeOid) -> Result<Option<Cell>, FdwError> {
    let value = row
        .as_object()
        .and_then(|row| row.get(name))
        .ok_or_else(|| format!("source column '{name}' not found"))?;
    // Atelier may encode NULL as an empty string; preserve text and JSON semantics.
    if value.is_null()
        || (value.as_str() == Some("") && !matches!(oid, TypeOid::String | TypeOid::Json))
    {
        return Ok(None);
    }
    let convert = || -> Result<Cell, String> {
        Ok(match oid {
            TypeOid::I8 => Cell::I8(integer(value)?),
            TypeOid::I16 => Cell::I16(integer(value)?),
            TypeOid::I32 => Cell::I32(integer(value)?),
            TypeOid::I64 => Cell::I64(integer(value)?),
            TypeOid::F32 => {
                let original = real(value)?;
                let number = original as f32;
                if !number.is_finite() || (original != 0.0 && number == 0.0) {
                    return Err("number out of f32 range".into());
                }
                Cell::F32(number)
            }
            TypeOid::F64 => Cell::F64(real(value)?),
            TypeOid::Numeric => Cell::Numeric(real(value)?),
            TypeOid::Bool => Cell::Bool(match value {
                Value::Bool(value) => *value,
                Value::Number(number) if number.as_i64() == Some(0) => false,
                Value::Number(number) if number.as_i64() == Some(1) => true,
                Value::String(text) if text == "0" || text == "false" => false,
                Value::String(text) if text == "1" || text == "true" => true,
                _ => return Err("expected boolean, 0 or 1".into()),
            }),
            TypeOid::String => Cell::String(match value {
                Value::String(text) => text.clone(),
                // Return structured JSON as text to bypass the host's JSONB precision limit.
                Value::Object(_) | Value::Array(_) => value.to_string(),
                _ => return Err("expected a string or structured JSON".into()),
            }),
            TypeOid::Json => {
                let json = match value.as_str() {
                    Some(text) => serde_json::from_str::<Value>(text)
                        .map_err(|e| format!("invalid JSON: {e}"))?,
                    None => value.clone(),
                };
                validate_host_json(&json)?;
                Cell::Json(json.to_string())
            }
            TypeOid::Date => {
                // IRIS dates count days since 1840-12-31; Unix epoch corresponds to day 47117.
                let seconds = if let Ok(days) = integer::<i64>(value) {
                    days.checked_sub(47117)
                        .and_then(|days| days.checked_mul(86400))
                        .filter(|seconds| DateTime::from_timestamp(*seconds, 0).is_some())
                        .ok_or("IRIS date out of range")?
                } else {
                    let text = value
                        .as_str()
                        .ok_or("expected YYYY-MM-DD or IRIS day count")?;
                    let date = NaiveDate::parse_from_str(text, "%Y-%m-%d")
                        .map_err(|e| format!("invalid date: {e}"))?;
                    date.and_hms_opt(0, 0, 0)
                        .ok_or("invalid midnight")?
                        .and_utc()
                        .timestamp()
                };
                Cell::Date(seconds)
            }
            TypeOid::Timestamp => Cell::Timestamp(timestamp(
                value.as_str().ok_or("expected timestamp string")?,
            )?),
            TypeOid::Timestamptz => Cell::Timestamptz(timestamp(
                value.as_str().ok_or("expected timestamp string")?,
            )?),
        })
    };
    convert()
        .map(Some)
        .map_err(|error| format!("column '{name}' ({oid:?}): {error}"))
}
