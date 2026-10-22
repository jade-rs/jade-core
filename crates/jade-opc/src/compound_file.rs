//! A writer for the Compound File Binary format ([MS-CFB]) that a `.doc` and
//! an encrypted Office package live in: version 3 (512-byte sectors),
//! storages holding streams.
//!
//! Written here rather than through a general CFB crate because those stamp
//! the entries with the current time, which `wasm32-unknown-unknown` has no
//! clock for (`SystemTime::now` panics there), and the browser build writes
//! these files too. Word leaves the times of a document's streams zero
//! anyway, as [MS-CFB] §2.6.1 allows.

const SECTOR: usize = 512;
const MINI_SECTOR: usize = 64;
/// Streams shorter than this live in the mini stream ([MS-CFB] §2.2).
const MINI_STREAM_CUTOFF: usize = 4096;
const ENTRIES_PER_SECTOR: usize = SECTOR / DIRECTORY_ENTRY;
const DIRECTORY_ENTRY: usize = 128;
const IDS_PER_SECTOR: usize = SECTOR / 4;
const HEADER_DIFAT: usize = 109;

const FREE: u32 = 0xFFFF_FFFF;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FAT_SECTOR: u32 = 0xFFFF_FFFD;
const DIFAT_SECTOR: u32 = 0xFFFF_FFFC;
const NO_STREAM: u32 = 0xFFFF_FFFF;

/// One entry of the directory being written.
struct Node {
    name: String,
    /// 1 storage, 2 stream, 5 root.
    kind: u8,
    /// The stream's contents (index into the caller's list).
    stream: Option<usize>,
    children: Vec<usize>,
    left: u32,
    right: u32,
    child: u32,
}

/// The bytes of a compound file holding `streams`, each a `(path, contents)`
/// pair: a `/`-separated path from the root storage (`"/WordDocument"`,
/// `"/\u{6}DataSpaces/Version"`), the storages on the way created as needed.
/// Each name is at most 31 UTF-16 units and every path is unique.
pub fn write(streams: &[(&str, &[u8])]) -> Result<Vec<u8>, String> {
    let mut nodes = vec![Node {
        name: "Root Entry".into(),
        kind: 5,
        stream: None,
        children: Vec::new(),
        left: NO_STREAM,
        right: NO_STREAM,
        child: NO_STREAM,
    }];
    for (index, (path, _)) in streams.iter().enumerate() {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let mut parent = 0;
        for (depth, part) in parts.iter().enumerate() {
            if part.is_empty() || part.encode_utf16().count() > 31 {
                return Err(format!("invalid compound file path {path:?}"));
            }
            let last = depth + 1 == parts.len();
            let existing = nodes[parent]
                .children
                .iter()
                .copied()
                .find(|&child| nodes[child].name == *part);
            parent = match existing {
                Some(child) if !last && nodes[child].kind == 1 => child,
                Some(_) => return Err(format!("duplicate compound file path {path:?}")),
                None => {
                    nodes.push(Node {
                        name: part.to_string(),
                        kind: if last { 2 } else { 1 },
                        stream: last.then_some(index),
                        children: Vec::new(),
                        left: NO_STREAM,
                        right: NO_STREAM,
                        child: NO_STREAM,
                    });
                    let id = nodes.len() - 1;
                    nodes[parent].children.push(id);
                    id
                }
            };
        }
    }
    let sectors_for = |bytes: usize| bytes.div_ceil(SECTOR);
    let is_mini = |bytes: &[u8]| !bytes.is_empty() && bytes.len() < MINI_STREAM_CUTOFF;

    // Small streams go, 64-byte mini sector by mini sector, into the mini
    // stream, itself stored in ordinary sectors under the root entry.
    let mut mini_stream = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut starts = vec![END_OF_CHAIN; streams.len()];
    for (index, (_, bytes)) in streams.iter().enumerate() {
        if !is_mini(bytes) {
            continue;
        }
        let first = mini_fat.len() as u32;
        let count = bytes.len().div_ceil(MINI_SECTOR) as u32;
        for offset in 0..count {
            mini_fat.push(if offset + 1 == count {
                END_OF_CHAIN
            } else {
                first + offset + 1
            });
        }
        starts[index] = first;
        mini_stream.extend_from_slice(bytes);
        mini_stream.resize(mini_fat.len() * MINI_SECTOR, 0);
    }

    let directory_sectors = nodes.len().div_ceil(ENTRIES_PER_SECTOR);
    let mini_fat_sectors = (mini_fat.len() * 4).div_ceil(SECTOR);
    let mini_stream_sectors = sectors_for(mini_stream.len());
    let stream_sectors: usize = streams
        .iter()
        .filter(|(_, bytes)| bytes.len() >= MINI_STREAM_CUTOFF)
        .map(|(_, bytes)| sectors_for(bytes.len()))
        .sum();
    let content = directory_sectors + mini_fat_sectors + mini_stream_sectors + stream_sectors;

    // Enough FAT sectors to describe every sector, themselves and the DIFAT
    // sectors that list FAT sectors beyond the header's 109 included.
    let mut fat_sectors: usize = 1;
    let difat_sectors = loop {
        let difat = fat_sectors
            .saturating_sub(HEADER_DIFAT)
            .div_ceil(IDS_PER_SECTOR - 1);
        if fat_sectors * IDS_PER_SECTOR >= content + fat_sectors + difat {
            break difat;
        }
        fat_sectors += 1;
    };
    let total = content + fat_sectors + difat_sectors;

    let mut fat = vec![FREE; fat_sectors * IDS_PER_SECTOR];
    for entry in fat.iter_mut().take(fat_sectors) {
        *entry = FAT_SECTOR;
    }
    for entry in fat.iter_mut().skip(fat_sectors).take(difat_sectors) {
        *entry = DIFAT_SECTOR;
    }
    let mut next = fat_sectors + difat_sectors;
    let mut chain = |fat: &mut Vec<u32>, count: usize| -> u32 {
        if count == 0 {
            return END_OF_CHAIN;
        }
        let first = next;
        for (sector, entry) in fat.iter_mut().enumerate().skip(first).take(count) {
            *entry = if sector + 1 == first + count {
                END_OF_CHAIN
            } else {
                sector as u32 + 1
            };
        }
        next += count;
        first as u32
    };
    let directory_start = chain(&mut fat, directory_sectors);
    let mini_fat_start = chain(&mut fat, mini_fat_sectors);
    let mini_stream_start = chain(&mut fat, mini_stream_sectors);
    for (index, (_, bytes)) in streams.iter().enumerate() {
        if bytes.len() >= MINI_STREAM_CUTOFF {
            starts[index] = chain(&mut fat, sectors_for(bytes.len()));
        }
    }
    debug_assert_eq!(next, total);

    // Each storage's children as one red-black tree (all black and
    // balanced, ordered as §2.6.4 compares names).
    for storage in 0..nodes.len() {
        let mut order = nodes[storage].children.clone();
        order.sort_by(|&a, &b| compare_names(&nodes[a].name, &nodes[b].name));
        nodes[storage].child = balanced_tree(&order, &mut nodes);
    }

    let mut directory = Vec::with_capacity(directory_sectors * SECTOR);
    for node in &nodes {
        let (start, size) = match (node.kind, node.stream) {
            (5, _) if !mini_stream.is_empty() => (mini_stream_start, mini_stream.len() as u64),
            (5, _) => (END_OF_CHAIN, 0),
            (2, Some(index)) => (starts[index], streams[index].1.len() as u64),
            _ => (0, 0),
        };
        directory.extend_from_slice(&directory_entry(
            &node.name,
            node.kind,
            (node.left, node.right, node.child),
            start,
            size,
        ));
    }
    while directory.len() < directory_sectors * SECTOR {
        directory.extend_from_slice(&directory_entry(
            "",
            0,
            (NO_STREAM, NO_STREAM, NO_STREAM),
            0,
            0,
        ));
    }

    let mut out = Vec::with_capacity((total + 1) * SECTOR);
    // Header (§2.2).
    out.extend_from_slice(&[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1]);
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&0x003Eu16.to_le_bytes());
    out.extend_from_slice(&0x0003u16.to_le_bytes());
    out.extend_from_slice(&0xFFFEu16.to_le_bytes());
    out.extend_from_slice(&9u16.to_le_bytes());
    out.extend_from_slice(&6u16.to_le_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(fat_sectors as u32).to_le_bytes());
    out.extend_from_slice(&directory_start.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(MINI_STREAM_CUTOFF as u32).to_le_bytes());
    out.extend_from_slice(&mini_fat_start.to_le_bytes());
    out.extend_from_slice(&(mini_fat_sectors as u32).to_le_bytes());
    let first_difat = if difat_sectors == 0 {
        END_OF_CHAIN
    } else {
        fat_sectors as u32
    };
    out.extend_from_slice(&first_difat.to_le_bytes());
    out.extend_from_slice(&(difat_sectors as u32).to_le_bytes());
    for index in 0..HEADER_DIFAT {
        let id = if index < fat_sectors {
            index as u32
        } else {
            FREE
        };
        out.extend_from_slice(&id.to_le_bytes());
    }
    debug_assert_eq!(out.len(), SECTOR);

    // FAT sectors.
    for id in &fat {
        out.extend_from_slice(&id.to_le_bytes());
    }
    // DIFAT sectors: 127 more FAT sector ids each, then the next one's.
    let mut remaining = (HEADER_DIFAT..fat_sectors).collect::<Vec<_>>().into_iter();
    for difat in 0..difat_sectors {
        for _ in 0..IDS_PER_SECTOR - 1 {
            let id = remaining.next().map_or(FREE, |sector| sector as u32);
            out.extend_from_slice(&id.to_le_bytes());
        }
        let link = if difat + 1 == difat_sectors {
            END_OF_CHAIN
        } else {
            (fat_sectors + difat + 1) as u32
        };
        out.extend_from_slice(&link.to_le_bytes());
    }
    out.extend_from_slice(&directory);
    let mut mini_fat_bytes: Vec<u8> = mini_fat.iter().flat_map(|id| id.to_le_bytes()).collect();
    mini_fat_bytes.resize(mini_fat_sectors * SECTOR, 0xFF);
    out.extend_from_slice(&mini_fat_bytes);
    pad_to_sector(&mut out, &mini_stream);
    for (_, bytes) in streams {
        if bytes.len() >= MINI_STREAM_CUTOFF {
            pad_to_sector(&mut out, bytes);
        }
    }
    debug_assert_eq!(out.len(), (total + 1) * SECTOR);
    Ok(out)
}

fn pad_to_sector(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(bytes);
    let padded = bytes.len().div_ceil(SECTOR) * SECTOR;
    out.resize(out.len() + padded - bytes.len(), 0);
}

/// [MS-CFB] §2.6.4: shorter names first, then by upper-cased UTF-16 units.
fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    let units = |name: &str| -> Vec<u16> { name.to_uppercase().encode_utf16().collect() };
    let (a, b) = (units(a), units(b));
    a.len().cmp(&b.len()).then_with(|| a.cmp(&b))
}

/// Links the directory entries `sorted` (in name order) into a balanced
/// binary tree through their siblings, returning its root's id.
fn balanced_tree(sorted: &[usize], nodes: &mut [Node]) -> u32 {
    if sorted.is_empty() {
        return NO_STREAM;
    }
    let middle = sorted.len() / 2;
    let node = sorted[middle];
    nodes[node].left = balanced_tree(&sorted[..middle], nodes);
    nodes[node].right = balanced_tree(&sorted[middle + 1..], nodes);
    node as u32
}

/// One 128-byte directory entry (§2.6.1); its creation and modification
/// times are left zero.
fn directory_entry(
    name: &str,
    kind: u8,
    (left, right, child): (u32, u32, u32),
    start: u32,
    size: u64,
) -> [u8; DIRECTORY_ENTRY] {
    let mut entry = [0u8; DIRECTORY_ENTRY];
    let units: Vec<u16> = name.encode_utf16().collect();
    for (index, unit) in units.iter().enumerate() {
        entry[index * 2..index * 2 + 2].copy_from_slice(&unit.to_le_bytes());
    }
    let name_length = if units.is_empty() {
        0
    } else {
        (units.len() as u16 + 1) * 2
    };
    entry[64..66].copy_from_slice(&name_length.to_le_bytes());
    entry[66] = kind;
    // Black.
    entry[67] = 1;
    entry[68..72].copy_from_slice(&left.to_le_bytes());
    entry[72..76].copy_from_slice(&right.to_le_bytes());
    entry[76..80].copy_from_slice(&child.to_le_bytes());
    entry[116..120].copy_from_slice(&start.to_le_bytes());
    entry[120..128].copy_from_slice(&size.to_le_bytes());
    entry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    fn read_back(bytes: Vec<u8>, name: &str) -> Vec<u8> {
        let mut file = cfb::CompoundFile::open(Cursor::new(bytes)).expect("a compound file");
        let mut stream = file.open_stream(name).expect("stream");
        let mut contents = Vec::new();
        stream.read_to_end(&mut contents).expect("read");
        contents
    }

    #[test]
    fn small_large_and_empty_streams_read_back() {
        let small: Vec<u8> = (0..300u32).map(|value| value as u8).collect();
        let large: Vec<u8> = (0..70_000u32).map(|value| (value % 251) as u8).collect();
        let bytes = write(&[
            ("/WordDocument", &large),
            ("/1Table", &small),
            ("/Data", &[]),
        ])
        .unwrap();
        assert_eq!(read_back(bytes.clone(), "/WordDocument"), large);
        assert_eq!(read_back(bytes.clone(), "/1Table"), small);
        assert!(read_back(bytes, "/Data").is_empty());
    }

    #[test]
    fn a_file_needing_difat_sectors_reads_back() {
        // More than 109 FAT sectors' worth: over 7MB.
        let big: Vec<u8> = (0..7_500_000u32).map(|value| (value % 253) as u8).collect();
        let bytes = write(&[("/Data", &big), ("/1Table", b"table")]).unwrap();
        assert_eq!(read_back(bytes.clone(), "/Data"), big);
        assert_eq!(read_back(bytes, "/1Table"), b"table");
    }

    #[test]
    fn nested_storages_read_back() {
        let bytes = write(&[
            ("/\u{6}DataSpaces/Version", b"version"),
            (
                "/\u{6}DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace",
                b"info",
            ),
            ("/\u{6}DataSpaces/DataSpaceMap", b"map"),
            ("/EncryptedPackage", &[7u8; 5000]),
        ])
        .unwrap();
        assert_eq!(
            read_back(
                bytes.clone(),
                "/\u{6}DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace"
            ),
            b"info"
        );
        assert_eq!(
            read_back(bytes.clone(), "/\u{6}DataSpaces/Version"),
            b"version"
        );
        assert_eq!(read_back(bytes, "/EncryptedPackage"), vec![7u8; 5000]);
        assert!(write(&[("/a", b"x"), ("/a", b"y")]).is_err());
    }
}
