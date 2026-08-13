use git2::{Cred, RemoteCallbacks};

fn test() {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|_user, username_from_url, allowed_types| {
        if allowed_types.contains(git2::CredentialType::SSH_KEY) {
            let user = username_from_url.unwrap_or("git");
            Cred::ssh_key_from_agent(user)
        } else if allowed_types.contains(git2::CredentialType::USER_PASS_PLAINTEXT) {
            Cred::default()
        } else if allowed_types.contains(git2::CredentialType::DEFAULT) {
            Cred::default()
        } else {
            Err(git2::Error::from_str("no credentials available"))
        }
    });
}
