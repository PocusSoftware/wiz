//! Quick connect targets as people type them.

/// Parse a quick connect target: `user@host`, `host:port`, `user@[::1]:2222`, or a pasted
/// `ssh -p 2222 user@host`. A missing user is `default_user`, a missing port 22.
pub fn parse_target(input: &str, default_user: &str) -> Result<(String, String, u16), String> {
    let mut words: Vec<&str> = input.split_whitespace().collect();
    if words.first() == Some(&"ssh") {
        words.remove(0);
    }
    let mut port = None;
    let mut target = None;
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if w == "-p" {
            let p = words.get(i + 1).ok_or("-p needs a port.")?;
            port = Some(p.parse::<u16>().map_err(|_| "Port must be 1-65535.")?);
            i += 2;
            continue;
        }
        if let Some(p) = w.strip_prefix("-p") {
            port = Some(p.parse::<u16>().map_err(|_| "Port must be 1-65535.")?);
        } else if w.starts_with('-') {
            return Err("Only the -p option is understood.".into());
        } else if target.replace(w).is_some() {
            return Err("Enter one target, like user@host:port.".into());
        }
        i += 1;
    }
    let target = target.ok_or("Enter a target, like user@host:port.")?;
    let target = target.strip_prefix("ssh://").unwrap_or(target);
    let (user, rest) = match target.rsplit_once('@') {
        Some((u, r)) => (u.to_string(), r),
        None => (default_user.to_string(), target),
    };
    let (host, p) = if let Some(inner) = rest.strip_prefix('[') {
        let (h, after) = inner
            .split_once(']')
            .ok_or("Missing ] after the IPv6 address.")?;
        match after.strip_prefix(':') {
            Some(p) => (h, Some(p)),
            None if after.is_empty() => (h, None),
            None => return Err("Unexpected text after the IPv6 address.".into()),
        }
    } else if rest.matches(':').count() == 1 {
        let (h, p) = rest.split_once(':').unwrap();
        (h, Some(p))
    } else {
        // No colon, or a bare IPv6 address.
        (rest, None)
    };
    if let Some(p) = p {
        let p = p.parse::<u16>().map_err(|_| "Port must be 1-65535.")?;
        if port.is_some_and(|q| q != p) {
            return Err("Two different ports given.".into());
        }
        port = Some(p);
    }
    Ok((user, host.to_string(), port.unwrap_or(22)))
}
