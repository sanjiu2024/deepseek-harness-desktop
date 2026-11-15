//! A real shell, driven through a real pseudo-terminal.
//!
//! The point is not that PowerShell works — it is that bytes go in, bytes come back, and the
//! reader hands them over without mangling multi-byte output. That last part is why the reader
//! emits `Vec<u8>` rather than `String`: a UTF-8 sequence can split across two reads.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use dsh_native::pty::{Pty, PtyEvent, PtySize};

#[test]
fn a_shell_echoes_what_it_is_given() {
    eprintln!("spawning");
    let (sender, receiver) = mpsc::channel();
    let pty = Pty::spawn(&std::env::temp_dir(), PtySize::default(), move |event| {
        let _ = sender.send(event);
    })
    .expect("spawn a shell");
    eprintln!("spawned");

    pty.write("echo DSH-PTY-OK\r\n".as_bytes())
        .expect("write");
    eprintln!("wrote");

    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(PtyEvent::Data(bytes)) => {
                // PowerShell asks the terminal where the cursor is () and blocks
                // until it hears back. A real emulator answers this; a test has to do it
                // itself, or the shell never paints and the marker never arrives.
                if bytes.windows(4).any(|window| window == b"[6n") {
                    pty.write(b"[1;1R").expect("answer cursor query");
                }
                seen.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&seen).contains("DSH-PTY-OK") {
                    pty.close();
                    return;
                }
            }
            Ok(PtyEvent::Exited(_)) => break,
            Err(_) => {}
        }
    }
    pty.close();
    panic!(
        "the shell never echoed the marker; got {} bytes: {:?}",
        seen.len(),
        String::from_utf8_lossy(&seen)
    );
}

#[test]
fn resizing_does_not_error() {
    let pty = Pty::spawn(&std::env::temp_dir(), PtySize::default(), |_| {}).expect("spawn");
    pty.resize(PtySize {
        rows: 40,
        cols: 120,
    })
    .expect("resize");
    pty.close();
}

#[test]
fn a_burst_of_output_is_coalesced() {
    // One event per read would flood a WebView; the test asserts the batching exists by
    // counting events for a known amount of output.
    let (sender, receiver) = mpsc::channel();
    let pty = Pty::spawn(&std::env::temp_dir(), PtySize::default(), move |event| {
        let _ = sender.send(event);
    })
    .expect("spawn a shell");

    // The marker is assembled by the shell rather than typed literally: the terminal echoes
    // what we write, so a literal marker is "seen" in the echo before any output arrives —
    // which is exactly how this test failed the first time (531 bytes, marker found).
    pty.write(
        b"$line = 'x' * 60; 1..500 | ForEach-Object { $line }; Write-Output ('DSH' + '-BURST' + '-END')\r\n",
    )
    .expect("write");

    let deadline = Instant::now() + Duration::from_secs(40);
    let mut events = 0usize;
    let mut bytes = 0usize;
    // Only the tail is kept: the marker can straddle two chunks.
    let mut tail: Vec<u8> = Vec::new();
    let mut ended = false;
    while Instant::now() < deadline {
        match receiver.recv_timeout(Duration::from_millis(300)) {
            Ok(PtyEvent::Data(chunk)) => {
                events += 1;
                bytes += chunk.len();
                // A real emulator answers the cursor-position query; writing it before the
                // command would instead land in the shell's input line as text.
                if chunk.windows(4).any(|window| window == b"[6n") {
                    pty.write(b"[1;1R").expect("answer cursor query");
                }
                tail.extend_from_slice(&chunk);
                let found = tail.windows(13).any(|window| window == b"DSH-BURST-END");
                // Keep the tail bounded; a burst would otherwise grow with the output.
                if tail.len() > 4096 {
                    let keep = tail.len() - 4096;
                    tail.drain(..keep);
                }
                if found {
                    ended = true;
                    break;
                }
            }
            Ok(PtyEvent::Exited(_)) => break,
            Err(_) => {}
        }
    }
    pty.close();
    assert!(ended, "never saw the end marker; {bytes} bytes in {events} events");
    assert!(bytes > 20_000, "expected the burst to actually happen, got {bytes} bytes");
    assert!(
        events < 200,
        "output should be coalesced, not one event per read: {events} events for {bytes} bytes",
    );
}
