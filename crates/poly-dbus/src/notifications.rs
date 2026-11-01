//! `org.freedesktop.Notifications` (the Desktop Notifications
//! Specification, version 1.2), the service's side: method calls decoded
//! into [`Request`]s, replies and signals built.
//!
//! - `Notify(app_name s, replaces_id u, app_icon s, summary s, body s,
//!   actions as, hints a{sv}, expire_timeout i) → id u`
//! - `CloseNotification(id u)`, then `NotificationClosed(id, reason)`
//! - `GetCapabilities() → as`
//! - `GetServerInformation() → (name s, vendor s, version s, spec_version s)`
//! - signals `NotificationClosed(id u, reason u)` and
//!   `ActionInvoked(id u, action_key s)`

use crate::bus::errors;
use crate::message::Message;
use crate::wire::Value;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

pub const NAME: &str = "org.freedesktop.Notifications";
pub const PATH: &str = "/org/freedesktop/Notifications";
pub const INTERFACE: &str = "org.freedesktop.Notifications";
/// The specification version implemented.
pub const SPEC_VERSION: &str = "1.2";

/// `NotificationClosed` reasons.
pub mod reason {
    pub const EXPIRED: u32 = 1;
    pub const DISMISSED: u32 = 2;
    pub const CLOSED: u32 = 3;
    pub const UNDEFINED: u32 = 4;
}

/// The `urgency` hint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Low,
    #[default]
    Normal,
    Critical,
}

/// A `Notify` call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notify {
    pub app_name: String,
    pub replaces_id: u32,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    /// (key, label) pairs.
    pub actions: Vec<(String, String)>,
    pub urgency: Urgency,
    pub category: Option<String>,
    /// The `transient` hint: not kept in the history.
    pub transient: bool,
    /// The `resident` hint: not removed when an action is invoked.
    pub resident: bool,
    /// Milliseconds; -1: the server's default; 0: never.
    pub expire_timeout: i32,
}

/// What a client asked the service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Notify(Notify),
    Close(u32),
    GetCapabilities,
    GetServerInformation,
    Introspect,
    Ping,
}

/// Decode a method call to the service: the request, or the error to
/// send back.
pub fn parse(call: &Message) -> Result<Request, alloc::boxed::Box<Message>> {
    let iface = call.interface.as_deref().unwrap_or(INTERFACE);
    let member = call.member.as_deref().unwrap_or("");
    let invalid = |what: &str| alloc::boxed::Box::new(Message::error(call, errors::INVALID_ARGS, what));
    match (iface, member) {
        (INTERFACE, "Notify") => {
            if call.signature() != "susssasa{sv}i" {
                return Err(invalid("Notify takes (susssasa{sv}i)"));
            }
            let s = |i: usize| call.body[i].as_str().unwrap_or("").into();
            let list: Vec<String> =
                call.body[5].as_array().unwrap_or(&[]).iter().filter_map(|v| v.as_str().map(String::from)).collect();
            let actions = list.chunks_exact(2).map(|p| (p[0].clone(), p[1].clone())).collect();
            let mut n = Notify {
                app_name: s(0),
                replaces_id: call.body[1].as_u32().unwrap_or(0),
                app_icon: s(2),
                summary: s(3),
                body: s(4),
                actions,
                expire_timeout: call.body[7].as_i32().unwrap_or(-1),
                ..Notify::default()
            };
            for h in call.body[6].as_array().unwrap_or(&[]) {
                let Value::DictEntry(k, v) = h else { continue };
                let v = v.unvariant();
                match k.as_str().unwrap_or("") {
                    "urgency" => {
                        n.urgency = match v.as_int() {
                            Some(0) => Urgency::Low,
                            Some(2) => Urgency::Critical,
                            _ => Urgency::Normal,
                        }
                    }
                    "category" => n.category = v.as_str().map(String::from),
                    "transient" => n.transient = v.as_int().unwrap_or(0) != 0,
                    "resident" => n.resident = v.as_int().unwrap_or(0) != 0,
                    _ => {}
                }
            }
            Ok(Request::Notify(n))
        }
        (INTERFACE, "CloseNotification") => match call.body.first().and_then(Value::as_u32) {
            Some(id) if call.body.len() == 1 => Ok(Request::Close(id)),
            _ => Err(invalid("CloseNotification takes (u)")),
        },
        (INTERFACE, "GetCapabilities") => Ok(Request::GetCapabilities),
        (INTERFACE, "GetServerInformation") => Ok(Request::GetServerInformation),
        ("org.freedesktop.DBus.Introspectable", "Introspect") => Ok(Request::Introspect),
        ("org.freedesktop.DBus.Peer", "Ping") => Ok(Request::Ping),
        _ => Err(alloc::boxed::Box::new(Message::error(
            call,
            errors::UNKNOWN_METHOD,
            "not a method of org.freedesktop.Notifications",
        ))),
    }
}

/// The reply to `Notify`: the notification's ID.
pub fn notify_reply(call: &Message, id: u32) -> Message {
    Message::method_return(call, vec![Value::Uint32(id)])
}

/// The reply to `GetCapabilities`.
pub fn capabilities_reply(call: &Message, caps: &[&str]) -> Message {
    Message::method_return(call, vec![Value::strings(caps)])
}

/// The reply to `GetServerInformation`.
pub fn server_information_reply(call: &Message, name: &str, vendor: &str, version: &str) -> Message {
    let body = vec![Value::str(name), Value::str(vendor), Value::str(version), Value::str(SPEC_VERSION)];
    Message::method_return(call, body)
}

/// The reply to `Introspect`: the interface as XML.
pub fn introspect_reply(call: &Message) -> Message {
    const XML: &str = "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n\
\"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n\
<node>\n <interface name=\"org.freedesktop.Notifications\">\n\
  <method name=\"Notify\"><arg type=\"s\" direction=\"in\"/><arg type=\"u\" direction=\"in\"/>\
<arg type=\"s\" direction=\"in\"/><arg type=\"s\" direction=\"in\"/><arg type=\"s\" direction=\"in\"/>\
<arg type=\"as\" direction=\"in\"/><arg type=\"a{sv}\" direction=\"in\"/><arg type=\"i\" direction=\"in\"/>\
<arg type=\"u\" direction=\"out\"/></method>\n\
  <method name=\"CloseNotification\"><arg type=\"u\" direction=\"in\"/></method>\n\
  <method name=\"GetCapabilities\"><arg type=\"as\" direction=\"out\"/></method>\n\
  <method name=\"GetServerInformation\"><arg type=\"s\" direction=\"out\"/><arg type=\"s\" direction=\"out\"/>\
<arg type=\"s\" direction=\"out\"/><arg type=\"s\" direction=\"out\"/></method>\n\
  <signal name=\"NotificationClosed\"><arg type=\"u\"/><arg type=\"u\"/></signal>\n\
  <signal name=\"ActionInvoked\"><arg type=\"u\"/><arg type=\"s\"/></signal>\n\
 </interface>\n</node>\n";
    Message::method_return(call, vec![Value::str(XML)])
}

/// The `NotificationClosed` signal.
pub fn closed_signal(id: u32, reason: u32) -> Message {
    Message::signal(PATH, INTERFACE, "NotificationClosed", vec![Value::Uint32(id), Value::Uint32(reason)])
}

/// The `ActionInvoked` signal.
pub fn action_signal(id: u32, key: &str) -> Message {
    Message::signal(PATH, INTERFACE, "ActionInvoked", vec![Value::Uint32(id), Value::str(key)])
}

/// A client's `Notify` call (what `notify-send` sends).
pub fn notify_call(n: &Notify) -> Message {
    let mut actions = Vec::new();
    for (k, l) in &n.actions {
        actions.push(k.as_str());
        actions.push(l.as_str());
    }
    let urgency = match n.urgency {
        Urgency::Low => 0,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    };
    let mut hints = vec![Value::DictEntry(
        alloc::boxed::Box::new(Value::str("urgency")),
        alloc::boxed::Box::new(Value::Variant(alloc::boxed::Box::new(Value::Byte(urgency)))),
    )];
    if let Some(c) = &n.category {
        hints.push(Value::DictEntry(
            alloc::boxed::Box::new(Value::str("category")),
            alloc::boxed::Box::new(Value::Variant(alloc::boxed::Box::new(Value::str(c)))),
        ));
    }
    let body = vec![
        Value::str(&n.app_name),
        Value::Uint32(n.replaces_id),
        Value::str(&n.app_icon),
        Value::str(&n.summary),
        Value::str(&n.body),
        Value::strings(&actions),
        Value::Array("{sv}".into(), hints),
        Value::Int32(n.expire_timeout),
    ];
    Message::method_call(Some(NAME), PATH, Some(INTERFACE), "Notify", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_round_trips_through_the_wire() {
        let n = Notify {
            app_name: "notify-send".into(),
            summary: "Hello".into(),
            body: "From a test client".into(),
            actions: vec![("default".into(), "Open".into())],
            urgency: Urgency::Critical,
            category: Some("test".into()),
            expire_timeout: 5000,
            ..Notify::default()
        };
        let mut call = notify_call(&n);
        call.sender = Some(":1.3".into());
        let call = Message::decode(&call.encode(9)).unwrap();
        assert_eq!(call.signature(), "susssasa{sv}i");
        assert_eq!(parse(&call), Ok(Request::Notify(n)));
        let r = notify_reply(&call, 4);
        assert_eq!((r.reply_serial, r.body.as_slice()), (Some(9), &[Value::Uint32(4)][..]));
    }

    #[test]
    fn other_methods() {
        let call = |m: &str, body: Vec<Value>| Message::method_call(Some(NAME), PATH, Some(INTERFACE), m, body);
        assert_eq!(parse(&call("CloseNotification", vec![Value::Uint32(3)])), Ok(Request::Close(3)));
        assert!(parse(&call("CloseNotification", vec![])).is_err());
        assert_eq!(parse(&call("GetCapabilities", vec![])), Ok(Request::GetCapabilities));
        let info = server_information_reply(&call("GetServerInformation", vec![]), "Poly", "Poly", "0.0.1");
        assert_eq!(info.signature(), "ssss");
        assert_eq!(info.body[3], Value::str("1.2"));
        let bad = parse(&call("Notify", vec![Value::str("x")])).unwrap_err();
        assert_eq!(bad.error_name.as_deref(), Some(errors::INVALID_ARGS));
        let unknown = parse(&call("Explode", vec![])).unwrap_err();
        assert_eq!(unknown.error_name.as_deref(), Some(errors::UNKNOWN_METHOD));
        let s = closed_signal(3, reason::DISMISSED);
        assert_eq!(
            (s.member.as_deref(), s.body.as_slice()),
            (Some("NotificationClosed"), &[Value::Uint32(3), Value::Uint32(2)][..])
        );
        assert_eq!(action_signal(3, "default").signature(), "us");
        assert!(introspect_reply(&call("Introspect", vec![])).body[0].as_str().unwrap().contains("Notify"));
    }
}
