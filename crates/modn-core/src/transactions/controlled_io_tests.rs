use super::{copy, digest_file, CHUNK_BYTES};
use crate::progress::{CancellationToken, Cancelled, OperationControl, ProgressEvent};
use std::io::{self, Read};

const LARGE: usize = 32 * CHUNK_BYTES;

/// A large member whose reader cancels the operation after its first chunk.
struct CancelAfterFirstRead {
    token: CancellationToken,
    remaining: usize,
    reads: usize,
}

impl Read for CancelAfterFirstRead {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reads += 1;
        if self.reads == 1 {
            self.token.cancel();
        }
        let read = buffer.len().min(self.remaining);
        buffer[..read].fill(7);
        self.remaining -= read;
        Ok(read)
    }
}

fn control(token: &CancellationToken) -> OperationControl {
    OperationControl::new(token.clone(), |_| {})
}

#[test]
fn a_cancel_during_one_large_member_stops_after_one_chunk() {
    let token = CancellationToken::new();
    let control = control(&token);
    let mut input = CancelAfterFirstRead {
        token: token.clone(),
        remaining: LARGE,
        reads: 0,
    };
    let mut output = Vec::new();

    let error =
        copy(&mut input, &mut output, LARGE as u64 + 1, &control).expect_err("cancelled copy");

    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
    assert_eq!(input.reads, 1);
    assert_eq!(output.len(), CHUNK_BYTES);
}

#[test]
fn a_cancel_after_the_commit_point_finishes_the_copy() {
    let token = CancellationToken::new();
    let control = control(&token);
    control
        .advance(ProgressEvent::Commit { package: None })
        .expect("commit point");
    let mut input = CancelAfterFirstRead {
        token: token.clone(),
        remaining: LARGE,
        reads: 0,
    };
    let mut output = Vec::new();

    let copied = copy(&mut input, &mut output, LARGE as u64 + 1, &control).expect("copy");

    assert_eq!(copied, LARGE as u64);
    assert_eq!(output.len(), LARGE);
}

#[test]
fn the_copy_stops_at_its_limit() {
    let control = OperationControl::default();
    let bytes = vec![1u8; 3 * CHUNK_BYTES];
    let mut output = Vec::new();

    let copied = copy(&mut bytes.as_slice(), &mut output, 100, &control).expect("copy");

    assert_eq!(copied, 100);
}

#[test]
fn a_cancelled_file_digest_reports_cancelled() {
    let temp = tempfile::tempdir().expect("temp");
    let path = temp.path().join("large.bin");
    std::fs::write(&path, vec![3u8; LARGE]).expect("file");
    let token = CancellationToken::new();
    let control = control(&token);
    assert_eq!(
        digest_file(&path, &control).expect("digest"),
        blake3::hash(&vec![3u8; LARGE]).to_hex().to_string()
    );

    token.cancel();
    let error = digest_file(&path, &control).expect_err("cancelled digest");

    assert!(error.downcast_ref::<Cancelled>().is_some(), "{error:#}");
}
