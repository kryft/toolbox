#[derive(Debug)]
pub struct Chunk<'a> {
    pub start: usize,
    pub end: usize,
    pub exclusive_start: usize,
    pub text: &'a str,
    pub line_starts: Vec<usize>,
    pub context_lines: usize,
}

fn snap_back(start: usize, line_table: &Vec<usize>) -> usize {
    let i = line_table.partition_point(|&x| x <= start);
    line_table[i - 1]
}

fn snap_forward(start: usize, line_table: &Vec<usize>) -> Option<usize> {
    let i = line_table.partition_point(|&x| x < start);
    line_table.get(i).copied()
}

pub fn default_chunk_bytes() -> usize {
    std::env::var("LLAMA_CHUNK_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(256 * 1024)
}

pub fn split<'a>(
    doc: &'a str,
    from: usize,
    limit: Option<usize>,
    chunk_bytes: usize,
    max_chunks: usize,
) -> Vec<Chunk<'a>> {
    let total = doc.len();
    let range_end =
        doc.floor_char_boundary(from.saturating_add(limit.unwrap_or(usize::MAX)).min(total));

    let mut chunks = Vec::<Chunk<'a>>::new();

    if range_end <= from {
        return chunks;
    }

    // if from is in the middle of a line, snap back to the beginning
    let from_snapped = doc.as_bytes()[..from]
        .iter()
        .rposition(|&b| b == b'\n')
        .map_or(0, |i| i + 1);

    let line_table = build_line_table(doc, from_snapped, range_end);

    let overlap = chunk_bytes / 8;

    let mut prev_end = from_snapped.saturating_add(chunk_bytes).min(range_end);
    prev_end = snap_forward(prev_end, &line_table).unwrap_or(range_end);
    let i = line_table.partition_point(|&x| x < from_snapped);
    let j = line_table.partition_point(|&x| x < prev_end);

    chunks.push(Chunk {
        start: from_snapped,
        end: prev_end,
        context_lines: 0,
        exclusive_start: from_snapped,
        text: &doc[from_snapped..prev_end],
        line_starts: line_table[i..j].to_vec(),
    });

    while prev_end < range_end && chunks.len() < max_chunks {
        let start = snap_back(prev_end - overlap, &line_table);
        let raw_end = prev_end
            .saturating_add(chunk_bytes - overlap)
            .min(range_end);
        let end = snap_forward(raw_end, &line_table).unwrap_or(range_end);
        let exclusive_start = prev_end;

        let i = line_table.partition_point(|&x| x < start);
        let j = line_table.partition_point(|&x| x < end);
        let e = line_table.partition_point(|&x| x < exclusive_start);

        chunks.push(Chunk {
            start: start,
            end: end,
            exclusive_start: exclusive_start,
            text: &doc[start..end],
            line_starts: line_table[i..j].to_vec(),
            context_lines: e - i,
        });

        prev_end = end;
    }

    chunks
}

fn build_line_table(doc: &str, range_start: usize, range_end: usize) -> Vec<usize> {
    let mut table = vec![range_start];
    for (i, _) in doc[range_start..range_end].match_indices('\n') {
        let line_start = range_start + i + 1;
        if line_start < range_end {
            table.push(line_start);
        }
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Doc of `n` lines, each `len` 'a's plus a trailing newline.
    fn uniform(n: usize, len: usize) -> String {
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(&"a".repeat(len));
            s.push('\n');
        }
        s
    }

    /// Independent oracle: line starts within [start, end).
    fn line_starts_in(doc: &str, start: usize, end: usize) -> Vec<usize> {
        let mut v = vec![start];
        for (i, _) in doc[start..end].match_indices('\n') {
            let ls = start + i + 1;
            if ls < end {
                v.push(ls);
            }
        }
        v
    }

    /// Invariants for a completed scan (final chunk reaches range_end).
    fn assert_invariants(doc: &str, chunks: &[Chunk], range_start: usize, range_end: usize) {
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].start, range_start);
        assert_eq!(chunks[0].exclusive_start, range_start);
        assert_eq!(chunks[0].context_lines, 0);
        for (n, c) in chunks.iter().enumerate() {
            if n > 0 {
                assert_eq!(
                    c.exclusive_start,
                    chunks[n - 1].end,
                    "exclusive zones tile, chunk {n}"
                );
            }
            assert!(c.start < c.end, "chunk {n} is non-empty");
            assert!(
                c.start >= range_start && c.end <= range_end,
                "chunk {n} within range"
            );
            assert_eq!(c.text, &doc[c.start..c.end], "chunk {n} text");
            assert_eq!(
                c.line_starts,
                line_starts_in(doc, c.start, c.end),
                "line_starts, chunk {n}"
            );
            assert_eq!(
                c.context_lines,
                c.line_starts.partition_point(|&x| x < c.exclusive_start),
                "context_lines counts lines, chunk {n}"
            );
            if c.start > 0 {
                assert_eq!(
                    doc.as_bytes()[c.start - 1],
                    b'\n',
                    "start line-aligned, chunk {n}"
                );
            }
            if c.end < range_end {
                // end is a line start, so the newline is the byte before it
                assert_eq!(
                    doc.as_bytes()[c.end - 1],
                    b'\n',
                    "end line-aligned, chunk {n}"
                );
            }
        }
        assert_eq!(
            chunks.last().unwrap().end,
            range_end,
            "final chunk reaches range_end"
        );
    }

    #[test]
    fn exclusive_zones_tile_exactly() {
        let doc = uniform(20, 7); // 20 lines x 8 bytes = 160 bytes
        let chunks = split(&doc, 0, None, 32, 64);
        assert_invariants(&doc, &chunks, 0, doc.len());
        let zones: usize = chunks.iter().map(|c| c.end - c.exclusive_start).sum();
        assert_eq!(
            zones,
            doc.len(),
            "exclusive zones cover the scan exactly once"
        );
    }

    #[test]
    fn starts_and_ends_are_line_aligned_on_ragged_doc() {
        let mut doc = String::new();
        for len in 1..=12 {
            doc.push_str(&"b".repeat(len));
            doc.push('\n');
        }
        let chunks = split(&doc, 0, None, 20, 64);
        assert_invariants(&doc, &chunks, 0, doc.len());
    }

    #[test]
    fn context_lines_counts_lines_not_bytes() {
        // line pitch 5, C = 48 -> O = 6 > pitch, so the overlap spans 2 lines
        let doc = uniform(30, 4);
        let chunks = split(&doc, 0, None, 48, 64);
        assert_invariants(&doc, &chunks, 0, doc.len());
        for c in chunks.iter().skip(1) {
            let overlap = &doc[c.start..c.exclusive_start];
            assert_eq!(c.context_lines, overlap.matches('\n').count());
            assert_eq!(c.context_lines, 2, "expected a two-line overlap");
        }
    }

    #[test]
    fn final_chunk_ends_exactly_at_range_end_even_mid_line() {
        let doc = uniform(10, 10); // 100 bytes, line pitch 11
        let chunks = split(&doc, 0, Some(37), 20, 64);
        assert_invariants(&doc, &chunks, 0, 37);
        assert_eq!(chunks.last().unwrap().end, 37, "final cut may be mid-line");
    }

    #[test]
    fn tail_smaller_than_overlap_is_carried_by_final_chunk() {
        let mut doc = String::new();
        doc.push_str(&"a".repeat(60));
        doc.push('\n');
        doc.push_str("bbbbb");
        doc.push('\n'); // 67 bytes; C = 64, O = 8, tail [64, 67) = 3 < O
        let chunks = split(&doc, 0, None, 64, 64);
        assert_eq!(chunks.len(), 1);
        assert_invariants(&doc, &chunks, 0, doc.len());
    }

    #[test]
    fn mid_line_from_snaps_back_to_line_start() {
        let doc = uniform(10, 8); // line pitch 9
        let chunks = split(&doc, 13, None, 20, 64); // inside line [9, 18)
        assert_invariants(&doc, &chunks, 9, doc.len());
        let chunks = split(&doc, 9, None, 20, 64); // exactly a line start
        assert_invariants(&doc, &chunks, 9, doc.len());
    }

    #[test]
    fn from_beyond_end_and_zero_limit_yield_no_chunks() {
        let doc = uniform(5, 10);
        assert!(split(&doc, 0, Some(0), 20, 64).is_empty());
        assert!(split(&doc, doc.len(), None, 20, 64).is_empty());
        assert!(split(&doc, doc.len() + 100, None, 20, 64).is_empty());
    }

    #[test]
    fn mid_character_from_does_not_panic_and_snaps_to_enclosing_line() {
        // "héé\n" = 6 bytes per line; byte 8 is mid-character in line 1 [6, 12)
        let doc = "héé\n".repeat(10);
        let chunks = split(&doc, 8, None, 20, 64);
        assert_invariants(&doc, &chunks, 6, doc.len());
    }

    #[test]
    fn zero_overlap_with_c_below_8_tiles_exactly() {
        // C = 4 < 8 -> O = 0: start_N must equal exclusive_start_N exactly,
        // which pins snap_back's <= (not <) predicate
        let doc = uniform(20, 1); // one char per line, pitch 2
        let chunks = split(&doc, 0, None, 4, 64);
        assert_invariants(&doc, &chunks, 0, doc.len());
        assert_eq!(chunks.len(), 10);
        for c in chunks.iter().skip(1) {
            assert_eq!(
                c.start, c.exclusive_start,
                "no overlap: start == exclusive_start"
            );
        }
    }

    #[test]
    fn max_chunks_stops_the_scan_early() {
        let doc = uniform(50, 10); // 500 bytes
        let chunks = split(&doc, 0, None, 20, 3);
        assert_eq!(chunks.len(), 3);
        for (n, c) in chunks.iter().enumerate() {
            if n > 0 {
                assert_eq!(c.exclusive_start, chunks[n - 1].end);
            }
        }
        assert!(
            chunks.last().unwrap().end < doc.len(),
            "stopped before range end"
        );
    }

    #[test]
    fn limit_none_and_huge_limit_agree() {
        let doc = uniform(5, 10);
        let a = split(&doc, 0, Some(usize::MAX), 20, 64);
        let b = split(&doc, 0, None, 20, 64);
        assert_eq!(a.len(), b.len());
        assert_eq!(a.last().unwrap().end, doc.len());
    }
}
