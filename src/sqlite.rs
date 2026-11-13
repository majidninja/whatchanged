//! A minimal read-only SQLite file reader, std only: enough to list the
//! rows of one table.
//!
//! Newer distros keep the login/boot record in a small SQLite database
//! (wtmpdb's `/var/lib/wtmpdb/wtmp.db`) instead of the old binary `wtmp`.
//! Reading it takes the file header, the table b-tree pages and the record
//! format; no SQL, no indexes, no write-ahead log. Every offset is
//! checked, so a damaged file gives an error and never a panic.

/// One column value. Floats and blobs aren't needed and read as `Other`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
    Other,
}

/// Most pages we are willing to walk for one table (wtmp.db is a few
/// pages; this stops loops in a damaged file).
const MAX_PAGES: usize = 100_000;

const CUT: &str = "database is cut short or damaged";

/// All rows of `table`, in rowid order, each as its column values.
/// An `INTEGER PRIMARY KEY` column reads as `Null` (SQLite keeps it as
/// the row id, not in the row).
pub fn read_table(data: &[u8], table: &str) -> Result<Vec<Vec<Value>>, String> {
    let db = Db::open(data)?;
    // Page 1 holds the schema table: (type, name, tbl_name, rootpage, sql).
    let root = db
        .rows(1)?
        .into_iter()
        .find_map(|row| match row.as_slice() {
            [
                Value::Text(kind),
                Value::Text(name),
                _,
                Value::Int(root),
                ..,
            ] if kind == "table" && name == table => Some(*root),
            _ => None,
        })
        .ok_or_else(|| format!("no table '{table}' in the database"))?;
    let root = usize::try_from(root).map_err(|_| CUT)?;
    db.rows(root)
}

struct Db<'a> {
    data: &'a [u8],
    page_size: usize,
    /// Page size minus the reserved bytes at the end of each page.
    usable: usize,
}

impl<'a> Db<'a> {
    fn open(data: &'a [u8]) -> Result<Self, String> {
        let head = data.get(..100).ok_or("not an SQLite database")?;
        if &head[..16] != b"SQLite format 3\0" {
            return Err("not an SQLite database".into());
        }
        let page_size = match u16::from_be_bytes([head[16], head[17]]) {
            1 => 65536,
            n => usize::from(n),
        };
        if page_size < 512 || !page_size.is_power_of_two() {
            return Err(CUT.into());
        }
        let usable = page_size - usize::from(head[20]);
        if usable < 480 {
            return Err(CUT.into());
        }
        if u32::from_be_bytes([head[56], head[57], head[58], head[59]]) > 1 {
            return Err("database text isn't UTF-8".into());
        }
        Ok(Db {
            data,
            page_size,
            usable,
        })
    }

    /// Page `number` (they count from 1).
    fn page(&self, number: usize) -> Result<&'a [u8], String> {
        let start = number
            .checked_sub(1)
            .and_then(|n| n.checked_mul(self.page_size))
            .ok_or(CUT)?;
        Ok(self.data.get(start..start + self.page_size).ok_or(CUT)?)
    }

    /// Every row of the table b-tree rooted at page `root`.
    fn rows(&self, root: usize) -> Result<Vec<Vec<Value>>, String> {
        let mut rows = Vec::new();
        let mut seen = 0;
        // Pages still to visit, last one first, so rows come out in order.
        let mut todo = vec![root];
        while let Some(number) = todo.pop() {
            seen += 1;
            if seen > MAX_PAGES {
                return Err(CUT.into());
            }
            let page = self.page(number)?;
            // Page 1 starts with the 100-byte file header.
            let base = if number == 1 { 100 } else { 0 };
            let kind = page[base];
            let cells = usize::from(be16(page, base + 3)?);
            let pointers = base + if kind == 5 { 12 } else { 8 };
            match kind {
                // Interior page: child pointers, left to right.
                5 => {
                    let mut children = Vec::with_capacity(cells + 1);
                    for i in 0..cells {
                        let cell = usize::from(be16(page, pointers + 2 * i)?);
                        children.push(be32(page, cell)? as usize);
                    }
                    children.push(be32(page, base + 8)? as usize);
                    todo.extend(children.into_iter().rev());
                }
                // Leaf page: the rows themselves.
                13 => {
                    for i in 0..cells {
                        let cell = usize::from(be16(page, pointers + 2 * i)?);
                        rows.push(record(&self.payload(page, cell)?)?);
                    }
                }
                _ => return Err(CUT.into()),
            }
        }
        Ok(rows)
    }

    /// The bytes of the row in the leaf cell at `cell`, following overflow
    /// pages when the row is too long for its page.
    fn payload(&self, page: &[u8], cell: usize) -> Result<Vec<u8>, String> {
        let (size, n) = varint(page, cell)?;
        let (_rowid, m) = varint(page, cell + n)?;
        let size = usize::try_from(size).map_err(|_| CUT)?;
        if size > self.data.len() {
            return Err(CUT.into());
        }
        let start = cell + n + m;
        let max_local = self.usable - 35;
        let local = if size <= max_local {
            size
        } else {
            let min_local = (self.usable - 12) * 32 / 255 - 23;
            let k = min_local + (size - min_local) % (self.usable - 4);
            if k <= max_local { k } else { min_local }
        };
        let mut out = page.get(start..start + local).ok_or(CUT)?.to_vec();
        if local < size {
            let mut next = be32(page, start + local)? as usize;
            let mut hops = 0;
            while out.len() < size {
                hops += 1;
                if next == 0 || hops > MAX_PAGES {
                    return Err(CUT.into());
                }
                let overflow = self.page(next)?;
                let take = (size - out.len()).min(self.usable - 4);
                out.extend_from_slice(&overflow[4..4 + take]);
                next = be32(overflow, 0)? as usize;
            }
        }
        Ok(out)
    }
}

/// Splits a record into its column values.
fn record(payload: &[u8]) -> Result<Vec<Value>, String> {
    let (header_len, mut pos) = varint(payload, 0)?;
    let header_len = usize::try_from(header_len).map_err(|_| CUT)?;
    if header_len > payload.len() {
        return Err(CUT.into());
    }
    let mut body = header_len;
    let mut values = Vec::new();
    while pos < header_len {
        let (kind, n) = varint(payload, pos)?;
        pos += n;
        let (len, value): (usize, fn(&[u8]) -> Value) = match kind {
            0 => (0, |_| Value::Null),
            1 => (1, int),
            2 => (2, int),
            3 => (3, int),
            4 => (4, int),
            5 => (6, int),
            6 => (8, int),
            7 => (8, |_| Value::Other),
            8 => (0, |_| Value::Int(0)),
            9 => (0, |_| Value::Int(1)),
            10 | 11 => return Err(CUT.into()),
            k if k % 2 == 0 => (((k - 12) / 2) as usize, |_| Value::Other),
            k => (((k - 13) / 2) as usize, |b| {
                Value::Text(String::from_utf8_lossy(b).into_owned())
            }),
        };
        let end = body.checked_add(len).ok_or(CUT)?;
        values.push(value(payload.get(body..end).ok_or(CUT)?));
        body = end;
    }
    Ok(values)
}

/// A big-endian two's complement integer of 1 to 8 bytes.
fn int(bytes: &[u8]) -> Value {
    let mut v: i64 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
    for &b in bytes {
        v = (v << 8) | i64::from(b);
    }
    Value::Int(v)
}

/// SQLite's variable-length integer at `pos`: (value, bytes used).
fn varint(data: &[u8], pos: usize) -> Result<(i64, usize), String> {
    let mut v: u64 = 0;
    for i in 0..9 {
        let b = *data.get(pos + i).ok_or(CUT)?;
        if i == 8 {
            return Ok((((v << 8) | u64::from(b)) as i64, 9));
        }
        v = (v << 7) | u64::from(b & 0x7f);
        if b & 0x80 == 0 {
            return Ok((v as i64, i + 1));
        }
    }
    unreachable!()
}

fn be16(data: &[u8], pos: usize) -> Result<u16, String> {
    let b = data.get(pos..pos + 2).ok_or(CUT)?;
    Ok(u16::from_be_bytes([b[0], b[1]]))
}

fn be32(data: &[u8], pos: usize) -> Result<u32, String> {
    let b = data.get(pos..pos + 4).ok_or(CUT)?;
    Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gzip::gunzip;

    /// Made with Python's sqlite3, 512-byte pages: a wtmpdb-style `wtmp`
    /// table of 400 rows (so the tree has interior pages), a second table,
    /// and one row long enough to spill into overflow pages.
    const MANY: &[u8] = include_bytes!("../tests/data/many.db.gz");

    fn many() -> Vec<u8> {
        gunzip(MANY, 1 << 24).unwrap()
    }

    #[test]
    fn reads_every_row_in_order() {
        let rows = read_table(&many(), "wtmp").unwrap();
        assert_eq!(rows.len(), 400);
        for (i, row) in rows.iter().enumerate() {
            let i = i as i64;
            let want = vec![
                Value::Null, // the INTEGER PRIMARY KEY
                Value::Int(if i % 50 == 0 { 1 } else { 3 }),
                Value::Text(format!("user{i}")),
                Value::Int(1_700_000_000_000_000 + i * 1_000_003),
                if i % 3 == 0 {
                    Value::Null
                } else {
                    Value::Int(1_700_000_000_000_000 + i * 1_000_003 + 5)
                },
                Value::Text(if i == 123 {
                    "x".repeat(3000)
                } else {
                    "~".into()
                }),
                Value::Text(format!("6.12.{i}-amd64")),
                Value::Null,
            ];
            assert_eq!(row, &want, "row {i}");
        }
    }

    #[test]
    fn reads_all_integer_sizes_and_other_types() {
        let rows = read_table(&many(), "odd").unwrap();
        let ints: Vec<Value> = [
            0,
            1,
            -1,
            127,
            -128,
            32767,
            -32768,
            8_388_607,
            -8_388_608,
            2_147_483_647,
            -2_147_483_648,
            140_737_488_355_327,
            -140_737_488_355_328,
            i64::MAX,
            i64::MIN,
        ]
        .into_iter()
        .map(Value::Int)
        .collect();
        let got: Vec<Value> = rows.iter().map(|r| r[0].clone()).collect();
        assert_eq!(got[..ints.len()], ints);
        assert_eq!(rows[ints.len()], [Value::Other, Value::Other]); // 1.5, a blob
        assert_eq!(rows[ints.len() + 1][1], Value::Text("héllo".into()));
    }

    #[test]
    fn missing_table_and_bad_files_are_errors() {
        let db = many();
        let err = read_table(&db, "nope").unwrap_err();
        assert!(err.contains("no table 'nope'"), "{err}");
        assert!(read_table(b"hello", "wtmp").is_err());
        assert!(read_table(&[0u8; 4096], "wtmp").is_err());
        // Cut anywhere, or with any byte of the first pages changed, it
        // must give rows or an error, never panic or hang.
        for cut in (0..db.len()).step_by(97) {
            let _ = read_table(&db[..cut], "wtmp");
        }
        for i in (0..db.len().min(4096)).step_by(3) {
            let mut bad = db.clone();
            bad[i] ^= 0xff;
            let _ = read_table(&bad, "odd");
        }
    }
}
