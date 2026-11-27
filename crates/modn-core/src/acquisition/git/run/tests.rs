use super::*;

#[test]
fn the_tail_keeps_only_the_last_limit_bytes() {
    let mut tail = Tail::default();
    let mut last = Vec::new();
    for index in 0..20 * 1024 {
        last = vec![b'a'; 1024];
        last[0] = b'a' + (index % 26) as u8;
        tail.push(&last);
    }
    assert_eq!(tail.bytes.len(), OUTPUT_TAIL_LIMIT);
    assert!(tail.truncated);
    assert_eq!(
        tail.bytes[OUTPUT_TAIL_LIMIT - 1024..],
        last[..],
        "the kept bytes are the last bytes written"
    );
}

#[test]
fn one_oversized_chunk_keeps_its_last_limit_bytes() {
    let mut tail = Tail::default();
    let chunk = vec![b'z'; OUTPUT_TAIL_LIMIT * 3];
    tail.push(&chunk[..OUTPUT_TAIL_LIMIT * 2]);
    tail.push(&chunk);
    assert_eq!(tail.bytes.len(), OUTPUT_TAIL_LIMIT);
    assert_eq!(tail.bytes, chunk[chunk.len() - OUTPUT_TAIL_LIMIT..]);
    assert!(tail.truncated);
}

#[test]
fn short_output_is_kept_whole_and_not_marked_truncated() {
    let mut tail = Tail::default();
    tail.push(b"fatal: unable to access the remote\n");
    assert!(!tail.truncated);
    assert_eq!(tail.bytes, b"fatal: unable to access the remote\n");
}

#[test]
fn a_low_speed_marker_split_across_two_reads_is_found() {
    let mut scan = LowSpeedScan::new();
    scan.feed(b"error: RPC failed; curl 28 Operation too sl");
    assert!(!scan.seen, "the first read ends inside the marker");
    scan.feed(b"ow. Less than 1 bytes/sec transferred the last 90");
    assert!(scan.seen, "the marker continued across the read boundary");
}

#[test]
fn the_second_low_speed_marker_is_found_in_any_case() {
    let mut scan = LowSpeedScan::new();
    scan.feed(b"fatal: BYTES/SEC TRANSFERRED the last 90 seconds");
    assert!(scan.seen);
}

#[test]
fn a_marker_before_a_flood_is_remembered_outside_the_tail() {
    let mut scan = LowSpeedScan::new();
    scan.feed(
        b"fatal: Operation too slow. Less than 1 bytes/sec transferred the last 90 seconds\n",
    );
    let flood = vec![b'x'; READ_CHUNK];
    for _ in 0..(20 * 1024 * 1024 / READ_CHUNK) {
        scan.feed(&flood);
    }
    assert!(scan.seen, "the classification survives the flood");
    assert!(!contains(&scan.carry, b"operation too slow"));
}

#[test]
fn kept_text_skips_a_split_character_at_the_start() {
    let mut bytes = vec![0x80, 0x80];
    bytes.extend_from_slice("remote: done".as_bytes());
    assert_eq!(kept_text(&bytes), "remote: done");
}

#[test]
fn kept_text_never_exceeds_the_limit() {
    let bytes = vec![0xff; OUTPUT_TAIL_LIMIT];
    assert!(kept_text(&bytes).len() <= OUTPUT_TAIL_LIMIT);
}
