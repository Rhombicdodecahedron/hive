use crate::{Handle, Phone, PhoneMemory, Serial};

/// Which phones to try for an account, best first, and why the others can't be used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub phones: Vec<Serial>,
    pub skipped: Vec<String>,
}

/// The phones to try for `handle`:
/// 1. the one it is bound to (config), when plugged in and usable;
/// 2. the ones whose TikTok already showed the account;
/// 3. every other usable phone, the ones known not to hold it last.
///
/// Busy phones (`busy`) are left out: they are posting something else.
pub fn choose_phones(
    handle: &Handle,
    phones: &[Phone],
    bound: Option<&Serial>,
    memory: &PhoneMemory,
    busy: &[Serial],
) -> Choice {
    let mut usable = Vec::new();
    let mut skipped = Vec::new();
    for p in phones {
        if busy.contains(&p.serial) {
            skipped.push(format!("{}: busy posting", p.label()));
        } else if let Some(b) = p.blocker() {
            skipped.push(format!("{}: {b}", p.label()));
        } else {
            usable.push(p);
        }
    }
    if let Some(b) = bound {
        if !phones.iter().any(|p| &p.serial == b) {
            skipped.insert(0, format!("@{handle}'s phone ({b}) isn't plugged in"));
        }
    }
    if phones.is_empty() {
        skipped.push("no phone plugged in".into());
    }
    usable.sort_by_key(|p| {
        let notes = memory.get(&p.serial);
        let seen = notes.is_some_and(|n| n.accounts.contains_key(handle));
        let missing = notes.is_some_and(|n| n.missing.contains_key(handle));
        let charging = p.status.as_ref().is_some_and(|s| s.charging);
        (Some(&p.serial) != bound, !seen, missing, !charging)
    });
    Choice { phones: usable.into_iter().map(|p| p.serial.clone()).collect(), skipped }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Lock, Status};

    fn phone(serial: &str, charging: bool, secure: bool) -> Phone {
        Phone {
            serial: Serial::new(serial).unwrap(),
            state: "device".into(),
            model: serial.to_owned(),
            status: Some(Status {
                tiktok: Some("com.zhiliaoapp.musically".into()),
                charging,
                lock: Lock { showing: Some(true), secure: Some(secure) },
                ..Status::default()
            }),
        }
    }

    #[test]
    fn bound_phone_then_known_then_the_rest() {
        let h = Handle::new("acc").unwrap();
        let phones = [phone("A", true, false), phone("B", true, false), phone("C", false, false)];
        let mut mem = PhoneMemory::default();
        mem.has(&Serial::new("C").unwrap(), &h, "t");
        mem.lacks(&Serial::new("A").unwrap(), &h, None, "t");
        let c = choose_phones(&h, &phones, None, &mem, &[]);
        let order: Vec<_> = c.phones.iter().map(Serial::as_str).collect();
        assert_eq!(order, ["C", "B", "A"]);
        let bound = Serial::new("A").unwrap();
        let c = choose_phones(&h, &phones, Some(&bound), &mem, &[]);
        assert_eq!(c.phones[0].as_str(), "A");
    }

    #[test]
    fn blocked_and_busy_phones_say_why() {
        let h = Handle::new("acc").unwrap();
        let phones = [phone("A", true, true), phone("B", true, false)];
        let c = choose_phones(&h, &phones, Some(&Serial::new("Z").unwrap()), &PhoneMemory::default(),
                              &[Serial::new("B").unwrap()]);
        assert!(c.phones.is_empty());
        assert_eq!(c.skipped.len(), 3);
        assert!(c.skipped[0].contains("isn't plugged in"));
    }
}
