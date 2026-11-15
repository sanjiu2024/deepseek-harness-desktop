//! Read the store of a real installation.
//!
//! Skips unless `DSH_DESKTOP_HOME` points at a kernel home, so a normal
//! `cargo test` stays hermetic while a real machine can be checked in one command:
//!
//! ```text
//! DSH_DESKTOP_HOME="…/.dev-home" cargo test -p dsh-store --test real_home -- --nocapture
//! ```

use dsh_store::SessionStore;

#[test]
fn reads_a_real_installation() {
    let Ok(home) = std::env::var("DSH_DESKTOP_HOME") else {
        eprintln!("skipping: DSH_DESKTOP_HOME is not set");
        return;
    };

    let store = SessionStore::new(&home);
    let sessions = store.list().expect("the store is readable");

    println!("sessions found: {}", sessions.len());
    for session in sessions.iter().take(5) {
        println!(
            "  {} · {} · {} events · updated {}",
            session.id, session.title, session.event_count, session.updated_at
        );
    }

    // Opening the newest one must work, and must replay as events.
    if let Some(newest) = sessions.first() {
        let history = store.load(&newest.id).expect("the newest session loads");
        assert_eq!(history.header.id, newest.id);
        assert_eq!(history.events.len(), newest.event_count);
        println!("loaded {} events from {}", history.events.len(), newest.id);
    }
}
