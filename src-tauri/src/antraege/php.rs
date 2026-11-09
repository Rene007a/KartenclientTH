//! Liest PHP-serialisierte Werte, z. B. `a:1:{i:0;s:5:"Hallo";}`.
//!
//! Die Antragsdaten stammen aus einer PHP-Anwendung, die Listen (Antragsteller,
//! Flurstücke …) in diesem Format in einzelnen Textspalten speichert.

#[derive(Debug, Clone, PartialEq)]
pub enum Php {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// PHP-Arrays sind geordnete Schlüssel/Wert-Listen
    Array(Vec<(Php, Php)>),
}

impl Php {
    /// Wert zu einem Schlüssel (nur bei Arrays)
    pub fn get(&self, key: &str) -> Option<&Php> {
        match self {
            Php::Array(items) => items.iter().find(|(k, _)| k.to_text() == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// Einfacher Wert als Text; Arrays werden lesbar zusammengefasst
    pub fn to_text(&self) -> String {
        match self {
            Php::Null => String::new(),
            Php::Bool(b) => if *b { "ja" } else { "nein" }.to_string(),
            Php::Int(i) => i.to_string(),
            Php::Float(f) => f.to_string(),
            Php::Str(s) => s.trim().to_string(),
            Php::Array(items) => {
                let sep = if items.iter().any(|(_, v)| matches!(v, Php::Array(_))) {
                    " | "
                } else {
                    ", "
                };
                items
                    .iter()
                    .map(|(_, v)| match v {
                        Php::Array(_) => v.person_or_record(),
                        _ => v.to_text(),
                    })
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(sep)
            }
        }
    }

    /// Person: "Herr Max Muster, Hauptstr. 1, 99084 Erfurt" – sonst alle Werte mit ", "
    fn person_or_record(&self) -> String {
        let f = |k: &str| self.get(k).map(|v| v.to_text()).unwrap_or_default();
        if self.get("name").is_some() {
            let name = join(" ", &[f("anrede"), f("name")]);
            let ort = join(" ", &[f("plz"), f("ort")]);
            join(", ", &[name, f("strasse"), ort])
        } else {
            self.to_text()
        }
    }
}

fn join(sep: &str, parts: &[String]) -> String {
    parts
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(sep)
}

/// Gibt `None` zurück, wenn der Text kein gültiger PHP-serialisierter Wert ist.
pub fn unserialize(s: &[u8]) -> Option<Php> {
    let mut i = 0;
    let v = read(s, &mut i)?;
    Some(v)
}

fn read(s: &[u8], i: &mut usize) -> Option<Php> {
    let t = *s.get(*i)?;
    match t {
        b'N' => {
            *i += 2; // "N;"
            Some(Php::Null)
        }
        b'b' | b'i' | b'd' => {
            let v = until(s, *i + 2, b';')?;
            *i += 2 + v.len() + 1;
            match t {
                b'b' => Some(Php::Bool(v == "1")),
                b'i' => v.parse().ok().map(Php::Int),
                _ => v.parse().ok().map(Php::Float),
            }
        }
        b's' => {
            // s:<Bytelänge>:"<Text>";
            let len_txt = until(s, *i + 2, b':')?;
            let len: usize = len_txt.parse().ok()?;
            let start = *i + 2 + len_txt.len() + 2; // ':' und '"'
            let bytes = s.get(start..start + len)?;
            *i = start + len + 2; // '"' und ';'
            Some(Php::Str(String::from_utf8_lossy(bytes).into_owned()))
        }
        b'a' => {
            // a:<Anzahl>:{<Schlüssel><Wert>...}
            let n_txt = until(s, *i + 2, b':')?;
            let n: usize = n_txt.parse().ok()?;
            *i += 2 + n_txt.len() + 2; // ':' und '{'
            let mut items = Vec::with_capacity(n.min(1000));
            for _ in 0..n {
                let k = read(s, i)?;
                let v = read(s, i)?;
                items.push((k, v));
            }
            *i += 1; // '}'
            Some(Php::Array(items))
        }
        _ => None,
    }
}

/// Text ab `start` bis (ausschließlich) zum Zeichen `end`
fn until(s: &[u8], start: usize, end: u8) -> Option<&str> {
    let rest = s.get(start..)?;
    let pos = rest.iter().position(|&c| c == end)?;
    std::str::from_utf8(&rest[..pos]).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liste_von_texten() {
        let v = unserialize(br#"a:2:{i:0;s:4:"12/3";i:1;s:6:"4/5 ab";}"#).unwrap();
        assert_eq!(v.to_text(), "12/3, 4/5 ab");
    }

    #[test]
    fn umlaute_bytelaenge() {
        // "Gößnitz" hat 7 Zeichen, aber 9 Bytes
        let v = unserialize("s:9:\"Gößnitz\";".as_bytes()).unwrap();
        assert_eq!(v, Php::Str("Gößnitz".into()));
    }

    #[test]
    fn personen() {
        let s = br#"a:1:{i:0;a:4:{s:6:"anrede";s:4:"Frau";s:4:"name";s:3:"Max";s:3:"plz";s:5:"99084";s:3:"ort";s:6:"Erfurt";}}"#;
        assert_eq!(unserialize(s).unwrap().to_text(), "Frau Max, 99084 Erfurt");
    }

    #[test]
    fn karte_mit_zahlen() {
        let s = br#"a:3:{s:2:"kb";i:51;s:3:"gmk";s:12:"Erfurt-Mitte";s:5:"gmkNR";s:4:"0101";}"#;
        let v = unserialize(s).unwrap();
        assert_eq!(v.get("kb"), Some(&Php::Int(51)));
        assert_eq!(v.get("gmk").unwrap().to_text(), "Erfurt-Mitte");
    }

    #[test]
    fn kaputt() {
        assert!(unserialize(b"kein php").is_none());
        assert!(unserialize(br#"s:10:"kurz";"#).is_none());
    }
}
