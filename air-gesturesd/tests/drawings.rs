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
    assert!(!dir.join(".ok.json.tmp").exists(), "an interrupted save's temp file is cleaned up");
    assert!(dir.join("notes.txt").exists());
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
    assert_eq!(json["tries"][0][0][1], serde_json::json!([0.0, 10.0]));
}
