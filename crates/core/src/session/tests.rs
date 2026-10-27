//! Tests for `session`.

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
    assert_eq!(kbd_answers(&prompts, "pw"), ["", "pw", ""]);
    assert_eq!(kbd_answers(&[prompt("Password: ", false)], "pw"), ["pw"]);
    // A prompt that echoes is never a password prompt.
    assert_eq!(kbd_answers(&[prompt("Name: ", true)], "pw"), [""]);
}
