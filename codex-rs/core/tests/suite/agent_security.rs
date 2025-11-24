use codex_core::agent::AgentRegistry;
use std::fs;
use std::os::unix::fs::symlink;
use tempfile::TempDir;

#[test]
fn test_symlinked_config_file_rejected() {
    // 1. Create a temp directory for our fake home/.codex
    let temp_dir = TempDir::new().unwrap();
    let codex_dir = temp_dir.path().join(".codex");
    fs::create_dir(&codex_dir).unwrap();

    // 2. Create a real agents.toml OUTSIDE the allowed directory (simulating /etc/passwd or similar)
    // In a real attack, this would be a sensitive file. Here we just check if it gets loaded.
    let secret_dir = TempDir::new().unwrap();
    let secret_file = secret_dir.path().join("secret_agents.toml");
    fs::write(
        &secret_file,
        r#"
        [hacker_agent]
        prompt = "You are a hacker."
        "#,
    )
    .unwrap();

    // 3. Create a symlink from .codex/agents.toml -> secret_agents.toml
    let symlink_path = codex_dir.join("agents.toml");
    symlink(&secret_file, &symlink_path).expect("Failed to create symlink");

    // 4. Initialize the registry using the custom path
    // This uses the new `from_paths` API we added for testing.
    let registry = AgentRegistry::from_paths(Some(codex_dir)).unwrap();

    // 5. Assert that the "hacker_agent" was NOT loaded.
    // If the symlink check works, it should log an error and return a registry with only the default agent.
    assert!(
        registry.get_agent("hacker_agent").is_none(),
        "Security bypass: Symlinked agents.toml was loaded!"
    );

    // 6. Verify default agent still exists
    assert!(
        registry.get_agent("general").is_some(),
        "Registry should still contain default agents"
    );
}

#[test]
fn test_symlinked_directory_rejected() {
    // 1. Create a directory that will be the target
    let real_dir = TempDir::new().unwrap();
    let real_codex = real_dir.path().join(".codex");
    fs::create_dir(&real_codex).unwrap();
    
    // Add a valid agent there
    fs::write(
        real_codex.join("agents.toml"),
        r#"
        [valid_agent]
        prompt = "Valid."
        "#,
    ).unwrap();

    // 2. Create a symlink TO that directory
    let link_dir = TempDir::new().unwrap();
    let symlinked_codex = link_dir.path().join("symlink_codex");
    symlink(&real_codex, &symlinked_codex).expect("Failed to create dir symlink");

    // 3. Try to load from the symlinked directory path
    let registry = AgentRegistry::from_paths(Some(symlinked_codex)).unwrap();

    // 4. Assert rejection
    assert!(
        registry.get_agent("valid_agent").is_none(),
        "Security bypass: Symlinked directory was followed!"
    );
}
