//! Значение реестра Windows в той форме, что нужна проверкам политик
//! (Claude, браузеры). Чтение — в win.rs; сама логика проверок — чистые
//! функции над этими значениями и тестируется на любой ОС.

#[derive(Debug, Clone, PartialEq)]
pub enum RegValue {
    Absent,
    /// REG_SZ.
    Str(String),
    /// Любой другой тип (DWORD, REG_EXPAND_SZ…) — для политик это «чужое».
    Other(String),
}

impl RegValue {
    pub fn exists(&self) -> bool {
        !matches!(self, RegValue::Absent)
    }

    pub fn is_str(&self, s: &str) -> bool {
        matches!(self, RegValue::Str(v) if v == s)
    }

    /// Значение из записи владения: `{"exists":…,"kind":…,"data":…}`.
    /// None — запись не той формы.
    pub fn from_backup(v: &serde_json::Value) -> Option<RegValue> {
        match v.get("exists")?.as_bool()? {
            false => Some(RegValue::Absent),
            true => match v.get("kind")?.as_str()? {
                "String" => Some(RegValue::Str(v.get("data")?.as_str()?.to_string())),
                kind => Some(RegValue::Other(kind.to_string())),
            },
        }
    }
}
