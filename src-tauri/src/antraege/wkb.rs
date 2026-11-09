//! Kleiner WKB-Leser (Well-Known Binary) – nur so viel, wie wir brauchen:
//! die Ausdehnung (Bounding Box) einer Geometrie für den räumlichen Index.
//!
//! MariaDB speichert Geometrien intern als 4 Byte SRID + Standard-WKB.

/// Entfernt die 4 Byte SRID am Anfang der MariaDB-Geometrie.
pub fn strip_mariadb_srid(raw: &[u8]) -> Option<&[u8]> {
    raw.get(4..).filter(|wkb| !wkb.is_empty())
}

/// Bounding Box `[minx, miny, maxx, maxy]` einer WKB-Geometrie.
/// `None`, wenn die Daten fehlerhaft sind oder keine Koordinaten enthalten.
pub fn bbox(wkb: &[u8]) -> Option<[f64; 4]> {
    let mut r = Reader { b: wkb, pos: 0, bbox: [f64::MAX, f64::MAX, f64::MIN, f64::MIN] };
    r.geometry()?;
    if r.pos != wkb.len() || r.bbox[0] > r.bbox[2] {
        return None;
    }
    Some(r.bbox)
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    bbox: [f64; 4],
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        let bytes = self.b.get(self.pos..self.pos + N)?.try_into().ok()?;
        self.pos += N;
        Some(bytes)
    }

    fn u32(&mut self, le: bool) -> Option<u32> {
        let b = self.take::<4>()?;
        Some(if le { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    fn point(&mut self, le: bool) -> Option<()> {
        let x = self.take::<8>()?;
        let y = self.take::<8>()?;
        let (x, y) = if le {
            (f64::from_le_bytes(x), f64::from_le_bytes(y))
        } else {
            (f64::from_be_bytes(x), f64::from_be_bytes(y))
        };
        if x.is_finite() && y.is_finite() {
            self.bbox = [self.bbox[0].min(x), self.bbox[1].min(y), self.bbox[2].max(x), self.bbox[3].max(y)];
        }
        Some(())
    }

    fn geometry(&mut self) -> Option<()> {
        let le = self.take::<1>()?[0] == 1;
        let geom_type = self.u32(le)?;
        match geom_type {
            1 => self.point(le),
            2 => {
                let n = self.u32(le)?;
                (0..n).try_for_each(|_| self.point(le))
            }
            3 => {
                let rings = self.u32(le)?;
                (0..rings).try_for_each(|_| {
                    let n = self.u32(le)?;
                    (0..n).try_for_each(|_| self.point(le))
                })
            }
            // Multi* und GeometryCollection: enthalten vollständige Geometrien
            4..=7 => {
                let n = self.u32(le)?;
                (0..n).try_for_each(|_| self.geometry())
            }
            _ => None, // 3D/Messwerte o. Ä. werden nicht unterstützt
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le_polygon(coords: &[(f64, f64)]) -> Vec<u8> {
        let mut v = vec![1u8];
        v.extend(3u32.to_le_bytes());
        v.extend(1u32.to_le_bytes());
        v.extend((coords.len() as u32).to_le_bytes());
        for (x, y) in coords {
            v.extend(x.to_le_bytes());
            v.extend(y.to_le_bytes());
        }
        v
    }

    #[test]
    fn polygon_und_collection() {
        let p = le_polygon(&[(1.0, 2.0), (5.0, 2.0), (5.0, 7.0), (1.0, 2.0)]);
        assert_eq!(bbox(&p), Some([1.0, 2.0, 5.0, 7.0]));

        let mut c = vec![1u8];
        c.extend(7u32.to_le_bytes());
        c.extend(2u32.to_le_bytes());
        c.extend(&p);
        c.extend(le_polygon(&[(-3.0, 0.0), (0.0, 10.0), (-3.0, 0.0)]));
        assert_eq!(bbox(&c), Some([-3.0, 0.0, 5.0, 10.0]));
    }

    #[test]
    fn fehlerhaft() {
        let p = le_polygon(&[(1.0, 2.0), (5.0, 2.0)]);
        assert_eq!(bbox(&p[..p.len() - 3]), None); // abgeschnitten
        assert_eq!(bbox(&[]), None);
        assert_eq!(strip_mariadb_srid(&[0, 0, 0, 0]), None);
    }
}
