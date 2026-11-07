//! `scripts/examples.sh` runs each example in `examples/`, as it lists them by hand.

use std::fs;
use std::path::Path;

#[test]
fn examples_script_runs_every_example() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = fs::read_to_string(root.join("scripts/examples.sh")).unwrap();
    let mut listed: Vec<&str> = script
        .split("--example ")
        .skip(1)
        .map(|rest| rest.split([' ', '\n']).next().unwrap())
        .collect();
    let mut examples: Vec<String> = fs::read_dir(root.join("examples"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();
    listed.sort_unstable();
    examples.sort_unstable();
    assert_eq!(listed, examples);
}
