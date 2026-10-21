//! The core against a real desktop agent, built with its stand-in for Windows
//! Hello (agent: `cargo build --features test-authenticator`), over a real
//! socket: turning unlock on, unlocking, another account, cancelling, turning
//! it off, and a program pretending to be the agent. Set PB_TEST_AGENT to that
//! build's `passbundle-agent`; without it, these tests say so and pass.

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use passbundle_core::agent::{Agent, Status};
use passbundle_core::crypto::SymmetricKey;
use passbundle_core::Error;

/// A private agent: its own home folder (so its own socket), and the stand-in's answers.
struct TestAgent {
    child: Child,
    home: PathBuf,
    program: PathBuf,
    address: PathBuf,
}

impl TestAgent {
    fn start() -> Option<TestAgent> {
        let Some(program) = std::env::var_os("PB_TEST_AGENT").map(PathBuf::from) else {
            eprintln!("PB_TEST_AGENT isn't set; skipping the agent tests.");
            return None;
        };
        // Short: Unix socket paths are limited to about 100 bytes, and macOS's temporary folder is long.
        let base = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let home = base.join(format!("pb-core-agent-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&home).unwrap();
        let child = Command::new(&program)
            .arg("--serve")
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("LOCALAPPDATA", &home)
            .env("PASSBUNDLE_AGENT_TEST_HOME", &home)
            .env("PASSBUNDLE_AGENT_TEST_AUTHENTICATOR", &home)
            .spawn()
            .expect("the agent starts");
        let address = passbundle_ipc::address_for(
            home.as_os_str(),
            passbundle_core::brand::NAME,
            passbundle_core::brand::DOMAIN,
        );
        let started = Instant::now();
        while passbundle_ipc::connect(&address).is_err() {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "the agent didn't start listening"
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        Some(TestAgent {
            child,
            home,
            program,
            address,
        })
    }

    /// The stand-in's next answers, one per prompt: true approves, false cancels.
    fn answers(&self, answers: &[bool]) {
        let text: Vec<&str> = answers
            .iter()
            .map(|a| if *a { "yes" } else { "no" })
            .collect();
        std::fs::write(self.home.join("answers"), text.join("\n")).unwrap();
    }

    fn prompts(&self) -> String {
        std::fs::read_to_string(self.home.join("prompts")).unwrap_or_default()
    }

    fn connect(&self, user_id: &str) -> Agent {
        Agent::connect(&self.address, &self.program, user_id).expect("connects to the agent")
    }
}

impl Drop for TestAgent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[test]
fn turns_on_unlocks_keeps_accounts_apart_cancels_and_turns_off() {
    let Some(agent) = TestAgent::start() else {
        return;
    };
    let user_key = SymmetricKey::generate();

    let mut ana = agent.connect("ana-user-id");
    assert_eq!(ana.status().unwrap(), Status::NotSetUp);
    // Nothing comes back for an account that hasn't turned it on, and nobody is asked.
    assert!(ana.unlock().unwrap().is_none());
    assert_eq!(agent.prompts(), "");

    agent.answers(&[true]);
    assert!(ana.enroll(&user_key, "ana@example.test").unwrap());
    assert_eq!(ana.status().unwrap(), Status::SetUp);
    assert!(
        agent
            .prompts()
            .contains("ana@example.test in the PassBundle app"),
        "{}",
        agent.prompts()
    );

    // A new connection, as the app's next unlock: the same key comes back.
    let mut later = agent.connect("ana-user-id");
    agent.answers(&[true]);
    assert_eq!(later.unlock().unwrap(), Some(user_key.clone()));
    assert!(
        agent
            .prompts()
            .contains("Unlock PassBundle in the PassBundle app"),
        "{}",
        agent.prompts()
    );

    // Another account gets nothing.
    let mut ben = agent.connect("ben-user-id");
    assert_eq!(ben.status().unwrap(), Status::NotSetUp);
    assert!(ben.unlock().unwrap().is_none());

    // Cancelling the prompt returns nothing and keeps it turned on.
    agent.answers(&[false]);
    assert!(later.unlock().unwrap().is_none());
    assert_eq!(later.status().unwrap(), Status::SetUp);

    // Turning it off.
    later.remove().unwrap();
    assert_eq!(later.status().unwrap(), Status::NotSetUp);
}

#[test]
fn refuses_a_program_pretending_to_be_the_agent() {
    if std::env::var_os("PB_TEST_AGENT").is_none() {
        eprintln!("PB_TEST_AGENT isn't set; skipping the agent tests.");
        return;
    }
    let base = if cfg!(unix) {
        PathBuf::from("/tmp")
    } else {
        std::env::temp_dir()
    };
    let home = base.join(format!("pb-core-fake-{}", uuid::Uuid::new_v4().simple()));
    let address = passbundle_ipc::address_for(
        home.as_os_str(),
        passbundle_core::brand::NAME,
        passbundle_core::brand::DOMAIN,
    );
    // This test program makes the socket first, as another program could while the agent isn't running.
    let listener = passbundle_ipc::Listener::bind(&address).unwrap();
    let accepting = std::thread::spawn(move || listener.accept().map(|_| ()));
    let agent = std::env::var_os("PB_TEST_AGENT")
        .map(PathBuf::from)
        .unwrap();
    let refused = Agent::connect(&address, &agent, "ana-user-id").err();
    assert!(matches!(refused, Some(Error::Refused(_))), "{refused:?}");
    let _ = accepting.join();
    let _ = std::fs::remove_dir_all(&home);
    // And with nothing listening at all, there's no agent.
    assert_eq!(
        Agent::connect(Path::new(&address), &agent, "ana-user-id").err(),
        Some(Error::NoAgent)
    );
}

#[test]
fn the_app_turns_it_on_unlocks_with_it_and_signing_out_turns_it_off() {
    use passbundle_core::app::{App, State};
    use passbundle_core::secrets::MemorySecretStore;
    use passbundle_core::store::{Settings, Store};

    let Some(agent) = TestAgent::start() else {
        return;
    };
    // The vectors' sample vault, kept on this device as if signed in.
    let v: serde_json::Value = serde_json::from_str(include_str!("vectors/core-v1.json")).unwrap();
    let pre = &v["account"]["pbkdf2"]["prelogin"];
    let dir = std::env::temp_dir().join(format!(
        "pb-core-app-agent-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let store = Store::new(&dir);
    store
        .save_settings(&Settings {
            server: "https://vault.example.test".into(),
            email: v["account"]["salt"].as_str().unwrap().into(),
            kdf: 0,
            kdf_iterations: pre["kdfIterations"].as_u64().unwrap() as u32,
            kdf_memory: None,
            kdf_parallelism: None,
            device_identifier: "d".into(),
        })
        .unwrap();
    store.save_sync(&v["sync"].to_string()).unwrap();
    let mut app = App::open(store, Box::new(MemorySecretStore::default()));
    app.use_agent_at(agent.address.clone(), agent.program.clone());

    assert_eq!(app.biometric_status().unwrap(), Status::NotSetUp);
    app.unlock(v["account"]["password"].as_str().unwrap())
        .unwrap();
    agent.answers(&[true]);
    assert!(app.enroll_biometric().unwrap());
    app.lock();
    assert!(matches!(app.state(), State::Locked { .. }));

    // Unlocked with the agent's key, no master password, no KDF.
    assert_eq!(app.biometric_status().unwrap(), Status::SetUp);
    agent.answers(&[true]);
    assert!(app.unlock_with_biometric().unwrap());
    assert!(matches!(app.state(), State::Unlocked { .. }));
    assert_eq!(app.vault().unwrap().items.len(), 4);

    app.sign_out();
    assert_eq!(
        Agent::connect(
            &agent.address,
            &agent.program,
            v["sync"]["profile"]["id"].as_str().unwrap()
        )
        .unwrap()
        .status()
        .unwrap(),
        Status::NotSetUp
    );
    let _ = std::fs::remove_dir_all(dir);
}
