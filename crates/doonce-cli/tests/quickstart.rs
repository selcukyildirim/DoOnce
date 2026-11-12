use std::process::Command;

#[test]
fn quickstart_doctor_and_config_work_end_to_end() {
    let dir = tempfile::tempdir().expect("temp dir");
    let bin = env!("CARGO_BIN_EXE_doonce");

    let quickstart = Command::new(bin)
        .env("DOONCE_HOME", dir.path())
        .arg("quickstart")
        .output()
        .expect("run quickstart");
    assert!(
        quickstart.status.success(),
        "quickstart failed: {}",
        String::from_utf8_lossy(&quickstart.stderr)
    );
    let stdout = String::from_utf8_lossy(&quickstart.stdout);
    assert!(stdout.contains("DoOnce Enterprise is ready"));
    assert!(stdout.contains("API key"));
    assert!(stdout.contains("dsk_"));

    let doctor = Command::new(bin)
        .env("DOONCE_HOME", dir.path())
        .arg("doctor")
        .output()
        .expect("run doctor");
    assert!(
        doctor.status.success(),
        "doctor failed: {}",
        String::from_utf8_lossy(&doctor.stderr)
    );
    assert!(String::from_utf8_lossy(&doctor.stdout).contains("All checks passed"));

    let config = Command::new(bin)
        .env("DOONCE_HOME", dir.path())
        .args(["config", "list"])
        .output()
        .expect("run config list");
    assert!(config.status.success());
    assert!(String::from_utf8_lossy(&config.stdout).contains("server.bind"));

    assert!(dir.path().join("doonce.db").exists());
    assert!(dir.path().join("master.key").exists());
}
