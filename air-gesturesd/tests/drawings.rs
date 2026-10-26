use air_gesturesd::drawings;
use band_core::controls::draw::Drawing;

fn line(length: f64) -> Vec<Vec<[f64; 2]>> {
    vec![vec![[0.0, 0.0], [0.0, length]]]
}

fn drawing(name: &str) -> Drawing {
    Drawing { name: name.into(), tries: vec![line(10.0), line(12.0), line(11.0)] }
}

#[test]
fn drawings_round_trip_by_name_and_can_be_deleted() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("drawings");
    assert_eq!(drawings::load(&dir), (Vec::new(), Vec::new()), "no directory: no drawings");
    assert_eq!(drawings::stamp(&dir), None);
    drawings::save(&dir, &drawing("z")).unwrap();
    drawings::save(&dir, &drawing("light bulb")).unwrap();
    let (loaded, problems) = drawings::load(&dir);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(loaded, [drawing("light bulb"), drawing("z")]);
    assert!(dir.join("z.json").exists());
    let before = drawings::stamp(&dir);
    assert!(before.is_some());
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(drawings::delete(&dir, "z").unwrap());
    assert!(!drawings::delete(&dir, "z").unwrap(), "already gone");
    assert_ne!(drawings::stamp(&dir), before, "a delete changes the stamp");
    assert_eq!(drawings::load(&dir).0, [drawing("light bulb")]);
}

#[test]
fn unreadable_files_are_skipped_and_described() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    drawings::save(dir, &drawing("ok")).unwrap();
    std::fs::write(dir.join("broken.json"), "{").unwrap();
    std::fs::write(dir.join("old.json"), r#"{"version":9,"tries":[]}"#).unwrap();
    std::fs::write(dir.join(".ok.json.tmp"), "{").unwrap();
    std::fs::write(dir.join("it's.json"), "{").unwrap();
    std::fs::write(dir.join("notes.txt"), "hi").unwrap();
    let (loaded, problems) = drawings::load(dir);
    assert_eq!(loaded, [drawing("ok")]);
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems.iter().any(|p| p.contains("broken.json")));
    assert!(problems.iter().any(|p| p.contains("version 9")));
    assert!(problems.iter().any(|p| p.contains("it's.json") && p.contains("name")), "{problems:?}");
    assert!(dir.join(".ok.json.tmp").exists(), "a save may be under way: load leaves its temp file alone");
    assert!(drawings::remove_stale(dir, std::time::Duration::from_secs(60)).is_empty());
    assert!(dir.join(".ok.json.tmp").exists(), "too fresh to be stale");
    assert!(drawings::remove_stale(dir, std::time::Duration::ZERO).is_empty());
    assert!(!dir.join(".ok.json.tmp").exists(), "an interrupted save's temp file is cleaned up");
    assert!(dir.join("notes.txt").exists() && dir.join("ok.json").exists());
}

#[test]
fn a_drawing_with_unreadable_tries_is_loaded_and_described() {
    let root = tempfile::tempdir().unwrap();
    let tiny = Drawing { name: "tiny".into(), tries: vec![vec![vec![[0.0, 0.0]]]] };
    drawings::save(root.path(), &tiny).unwrap();
    let (loaded, problems) = drawings::load(root.path());
    assert_eq!(loaded, [tiny]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("tiny") && problems[0].contains("teach it again"), "{problems:?}");
}

#[test]
fn names_are_checked() {
    for bad in ["", ".hidden", "../up", "it's", "a/b"] {
        assert!(drawings::check_name(bad).is_err(), "{bad:?}");
    }
    assert!(drawings::check_name("light bulb-2.v").is_ok());
    let root = tempfile::tempdir().unwrap();
    assert!(drawings::save(root.path(), &drawing("../x")).is_err());
}

#[test]
fn the_file_is_versioned_json_of_strokes() {
    let root = tempfile::tempdir().unwrap();
    drawings::save(root.path(), &drawing("z")).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(root.path().join("z.json")).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["frame"], 2, "drawn in band-core's frame since 2026-10-26");
    assert_eq!(json["tries"][0][0][1], serde_json::json!([0.0, 10.0]));
}

#[test]
fn loading_while_saving_never_loses_a_save() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().to_path_buf();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let loader = {
        let (dir, done) = (dir.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(std::sync::atomic::Ordering::Relaxed) {
                drawings::load(&dir);
            }
        })
    };
    let failed = (0..2000).filter(|_| drawings::save(&dir, &drawing("z")).is_err()).count();
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    loader.join().unwrap();
    assert_eq!(failed, 0, "saves lost to a concurrent load");
}

/// Templates taught before band-core turned its drawings the right way up (no `frame`): the
/// projection is the old one negated, so their points are negated once, the file rewritten
/// with `"frame": 2` and the original kept as `<name>.json.bak`.
#[test]
fn templates_from_before_the_frame_change_are_turned_over_once() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    let old = r#"{"version":1,"tries":[[[[0.0,0.0],[0.0,10.0]]],[[[1.0,2.0],[3.0,-4.0]]]]}"#;
    std::fs::write(dir.join("z.json"), old).unwrap();
    drawings::save(dir, &drawing("new")).unwrap();
    let loaded = drawings::load_all(dir);
    assert!(loaded.problems.is_empty(), "{:?}", loaded.problems);
    assert_eq!(loaded.migrated, ["z"]);
    let z = loaded.drawings.iter().find(|d| d.name == "z").unwrap();
    assert_eq!(z.tries, [vec![vec![[0.0, 0.0], [0.0, -10.0]]], vec![vec![[-1.0, -2.0], [-3.0, 4.0]]]]);
    assert_eq!(loaded.drawings.iter().find(|d| d.name == "new").unwrap(), &drawing("new"), "already in the new frame");
    assert_eq!(std::fs::read_to_string(dir.join("z.json.bak")).unwrap(), old);
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("z.json")).unwrap()).unwrap();
    assert_eq!(json["frame"], 2);
    let again = drawings::load_all(dir);
    assert!(again.migrated.is_empty(), "once");
    assert_eq!(again.drawings, loaded.drawings);
}

#[test]
fn a_template_that_cant_be_turned_over_on_disk_says_teach_it_again() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("drawings");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("z.json"), r#"{"version":1,"tries":[[[[0.0,0.0],[0.0,10.0]]]]}"#).unwrap();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    // Root (as in CI containers) writes into a read-only directory anyway: nothing to test.
    if std::fs::write(dir.join(".probe"), "").is_ok() {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        eprintln!("skipped: this user can write into a read-only directory");
        return;
    }
    let loaded = drawings::load_all(&dir);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(loaded.problems.iter().any(|p| p.contains("teach z again")), "{:?}", loaded.problems);
    assert_eq!(loaded.drawings[0].tries, [vec![vec![[0.0, 0.0], [0.0, -10.0]]]], "still used, the right way up");
    assert!(loaded.migrated.is_empty());
}

#[test]
fn a_template_from_a_newer_frame_is_skipped() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("z.json"), r#"{"version":1,"frame":3,"tries":[]}"#).unwrap();
    let loaded = drawings::load_all(root.path());
    assert!(loaded.drawings.is_empty());
    assert!(loaded.problems.iter().any(|p| p.contains("frame 3")), "{:?}", loaded.problems);
}
