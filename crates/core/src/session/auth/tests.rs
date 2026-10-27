//! Tests for `auth`.

use super::*;

fn prompt(text: &str, echo: bool) -> client::Prompt {
    client::Prompt {
        prompt: text.into(),
        echo,
    }
}

#[test]
fn password_only_answers_the_first_hidden_prompt() {
    let prompts = [
        prompt("Username: ", true),
        prompt("Password: ", false),
        prompt("Verification code: ", false),
    ];
    assert_eq!(split_round(&prompts, true), (Some(1), vec![0, 2]));
    // Once the password is used, everything goes to the user.
    assert_eq!(split_round(&prompts, false), (None, vec![0, 1, 2]));
    assert_eq!(
        split_round(&[prompt("Password: ", false)], true),
        (Some(0), vec![])
    );
    // A prompt that echoes is never a password prompt.
    assert_eq!(
        split_round(&[prompt("Name: ", true)], true),
        (None, vec![0])
    );
}
