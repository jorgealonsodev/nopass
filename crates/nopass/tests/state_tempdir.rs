//! `state::read` against a real filesystem — `Layout::under(TempDir)`, the
//! same unprivileged-test injection point M1 built (design.md §1).

use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use nopass::state::{self, FileReading, ReadFault};
use nopass_core::paths::Layout;

#[test]
fn a_missing_state_file_reads_as_absent_never_as_a_fault() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::under(dir.path());
    let path = layout.state_path(1000);

    assert_eq!(state::read(&path), FileReading::Absent);
}

#[test]
fn an_unreadable_directory_reads_as_faulted_io() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::under(dir.path());
    let path = layout.state_path(1000);
    let run_dir = path.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(&run_dir).unwrap();
    std::fs::write(&path, b"{}").unwrap();

    let original = std::fs::metadata(&run_dir).unwrap().permissions();
    let mut locked = original.clone();
    locked.set_mode(0o000);
    std::fs::set_permissions(&run_dir, locked).unwrap();

    let result = state::read(&path);

    // Restore permissions immediately, before any assertion can panic and
    // leak an unreadable temp directory past the test.
    std::fs::set_permissions(&run_dir, original).unwrap();

    assert_eq!(result, FileReading::Faulted(ReadFault::Io));
}

#[test]
fn rename_in_place_while_reading_never_produces_a_torn_parse() {
    let dir = tempfile::tempdir().unwrap();
    let layout = Layout::under(dir.path());
    let path = layout.state_path(1000);
    let tmp_path = layout.state_tmp_path(1000);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();

    let content_a = br#"{"schema":1,"uid":1000,"user":"a","active":true,"expires":null,"rule_path":"x","updated_at":1}"#.to_vec();
    let content_b = br#"{"schema":1,"uid":1000,"user":"b","active":false,"expires":null,"rule_path":"x","updated_at":2}"#.to_vec();

    let writer_tmp = tmp_path.clone();
    let writer_path = path.clone();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_millis(300);
        let mut toggle = false;
        while Instant::now() < deadline {
            let content = if toggle { &content_b } else { &content_a };
            std::fs::write(&writer_tmp, content).unwrap();
            std::fs::rename(&writer_tmp, &writer_path).unwrap();
            toggle = !toggle;
        }
    });

    let deadline = Instant::now() + Duration::from_millis(300);
    let mut observed_parsed = false;
    while Instant::now() < deadline {
        match state::read(&path) {
            FileReading::Absent => {}
            FileReading::Parsed(status) => {
                observed_parsed = true;
                assert!(
                    status.user == "a" || status.user == "b",
                    "torn parse produced an unexpected user: {}",
                    status.user
                );
            }
            FileReading::Faulted(fault) => panic!("torn parse observed: {fault:?}"),
        }
    }

    writer.join().unwrap();
    assert!(
        observed_parsed,
        "the reader loop must observe at least one successful parse to prove anything"
    );
}
