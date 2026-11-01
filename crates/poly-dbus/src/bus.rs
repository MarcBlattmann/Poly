//! The message bus (the D-Bus Specification, "Message Bus Specification"):
//! connections authenticate, say `Hello` and get a unique name (`:1.N`),
//! may own well-known names, and the bus routes their messages by the
//! `DESTINATION` field, filling in `SENDER`; signals without a destination
//! go to every connection with a matching rule (`AddMatch`). The bus
//! driver, `org.freedesktop.DBus`, answers the methods a client library
//! calls on connecting and watching names.
//!
//! Services of the bus's own process are **local**: they own names like
//! any connection ([`Bus::request_local_name`]), method calls to them
//! queue up for the caller ([`Bus::take_local`]), and what they send goes
//! out through [`Bus::send_local`]. That is how the compositor serves
//! `org.freedesktop.Notifications` on the bus it hosts.
//!
//! Not here: activation of services from `.service` files, the system
//! bus's policies, descriptor passing, eavesdropping and monitoring.

use crate::auth::{Progress, ServerAuth};
use crate::message::{Message, MessageType, Stream};
use crate::wire::Value;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

/// A connection to the bus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConnId(pub u32);

/// The bus's own process: local services.
pub const LOCAL: ConnId = ConnId(0);

/// The bus driver's name, path and interface.
pub const DRIVER: &str = "org.freedesktop.DBus";
pub const DRIVER_PATH: &str = "/org/freedesktop/DBus";

/// Error names of the specification.
pub mod errors {
    pub const SERVICE_UNKNOWN: &str = "org.freedesktop.DBus.Error.ServiceUnknown";
    pub const NAME_HAS_NO_OWNER: &str = "org.freedesktop.DBus.Error.NameHasNoOwner";
    pub const UNKNOWN_METHOD: &str = "org.freedesktop.DBus.Error.UnknownMethod";
    pub const INVALID_ARGS: &str = "org.freedesktop.DBus.Error.InvalidArgs";
    pub const MATCH_RULE_INVALID: &str = "org.freedesktop.DBus.Error.MatchRuleInvalid";
    pub const MATCH_RULE_NOT_FOUND: &str = "org.freedesktop.DBus.Error.MatchRuleNotFound";
    pub const ACCESS_DENIED: &str = "org.freedesktop.DBus.Error.AccessDenied";
    pub const NOT_SUPPORTED: &str = "org.freedesktop.DBus.Error.NotSupported";
}

/// A match rule (`AddMatch`): every key given must match.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatchRule {
    pub text: String,
    pub kind: Option<MessageType>,
    pub sender: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub path: Option<String>,
    pub path_namespace: Option<String>,
    pub destination: Option<String>,
    /// `argN`: the Nth body argument, a string equal to this.
    pub args: Vec<(usize, String)>,
}

impl MatchRule {
    /// Parse `key='value',...` (values quoted with `'`; outside quotes `\'`
    /// is an apostrophe).
    pub fn parse(text: &str) -> Option<MatchRule> {
        let mut rule = MatchRule { text: text.into(), ..MatchRule::default() };
        let mut chars = text.chars().peekable();
        loop {
            while chars.peek() == Some(&' ') {
                chars.next();
            }
            if chars.peek().is_none() {
                break;
            }
            let key: String = chars.by_ref().take_while(|&c| c != '=').collect();
            let mut value = String::new();
            let mut quoted = false;
            while let Some(c) = chars.next() {
                match c {
                    '\'' => quoted = !quoted,
                    '\\' if !quoted && chars.peek() == Some(&'\'') => {
                        chars.next();
                        value.push('\'');
                    }
                    ',' if !quoted => break,
                    c => value.push(c),
                }
            }
            if quoted {
                return None;
            }
            match key.trim() {
                "type" => {
                    rule.kind = Some(match value.as_str() {
                        "method_call" => MessageType::MethodCall,
                        "method_return" => MessageType::MethodReturn,
                        "error" => MessageType::Error,
                        "signal" => MessageType::Signal,
                        _ => return None,
                    })
                }
                "sender" => rule.sender = Some(value),
                "interface" => rule.interface = Some(value),
                "member" => rule.member = Some(value),
                "path" => rule.path = Some(value),
                "path_namespace" => rule.path_namespace = Some(value),
                "destination" => rule.destination = Some(value),
                "eavesdrop" => {}
                k if k.starts_with("arg") => {
                    // argN and argNpath: only plain string arguments here.
                    let n: usize = k[3..].trim_end_matches("path").trim_end_matches("namespace").parse().ok()?;
                    if n > 63 {
                        return None;
                    }
                    rule.args.push((n, value));
                }
                _ => return None,
            }
        }
        Some(rule)
    }

    /// Whether `m`, sent by a connection known by `sender_names` (its
    /// unique name and the well-known names it owns), matches.
    pub fn matches(&self, m: &Message, sender_names: &[String]) -> bool {
        let eq = |want: &Option<String>, have: &Option<String>| want.as_ref().is_none_or(|w| have.as_ref() == Some(w));
        self.kind.is_none_or(|k| k == m.kind)
            && self.sender.as_ref().is_none_or(|s| sender_names.contains(s))
            && eq(&self.interface, &m.interface)
            && eq(&self.member, &m.member)
            && eq(&self.path, &m.path)
            && eq(&self.destination, &m.destination)
            && self.path_namespace.as_ref().is_none_or(|ns| {
                m.path.as_ref().is_some_and(|p| {
                    ns == "/" || p == ns || (p.starts_with(ns.as_str()) && p.as_bytes().get(ns.len()) == Some(&b'/'))
                })
            })
            && self.args.iter().all(|(n, v)| m.body.get(*n).and_then(Value::as_str) == Some(v.as_str()))
    }
}

#[derive(Debug)]
struct Conn {
    /// Until `BEGIN`.
    auth: Option<ServerAuth>,
    stream: Stream,
    out: Vec<u8>,
    unique: Option<String>,
    matches: Vec<MatchRule>,
    failed: Option<String>,
}

/// The bus.
#[derive(Debug)]
pub struct Bus {
    guid: String,
    conns: BTreeMap<ConnId, Conn>,
    next_conn: u32,
    next_unique: u32,
    /// Unique and well-known names to their owners.
    names: BTreeMap<String, ConnId>,
    /// The serial of the last message the bus itself (driver or local
    /// services) sent.
    serial: u32,
    local_unique: String,
    local_inbox: Vec<Message>,
}

impl Bus {
    /// A bus with this GUID (32 hexadecimal digits). Its local services
    /// have the unique name `:1.0`.
    pub fn new(guid: &str) -> Bus {
        let local_unique = String::from(":1.0");
        let mut names = BTreeMap::new();
        names.insert(local_unique.clone(), LOCAL);
        Bus {
            guid: guid.into(),
            conns: BTreeMap::new(),
            next_conn: 1,
            next_unique: 1,
            names,
            serial: 0,
            local_unique,
            local_inbox: Vec::new(),
        }
    }

    pub fn guid(&self) -> &str {
        &self.guid
    }

    /// A new connection, its peer's user ID from the socket if known.
    pub fn connect(&mut self, peer_uid: Option<u32>) -> ConnId {
        let id = ConnId(self.next_conn);
        self.next_conn += 1;
        let conn = Conn {
            auth: Some(ServerAuth::new(&self.guid, peer_uid)),
            stream: Stream::default(),
            out: Vec::new(),
            unique: None,
            matches: Vec::new(),
            failed: None,
        };
        self.conns.insert(id, conn);
        id
    }

    /// Why a connection was cut off, if it was: send what is queued, then
    /// close it.
    pub fn failure(&self, id: ConnId) -> Option<&str> {
        self.conns.get(&id)?.failed.as_deref()
    }

    /// The bytes to send to a connection.
    pub fn take_output(&mut self, id: ConnId) -> Vec<u8> {
        let Some(c) = self.conns.get_mut(&id) else { return Vec::new() };
        let mut out = c.auth.as_mut().map(ServerAuth::take_output).unwrap_or_default();
        out.append(&mut c.out);
        out
    }

    /// The connections with something to send.
    pub fn pending(&self) -> Vec<ConnId> {
        self.conns.iter().filter(|(_, c)| !c.out.is_empty()).map(|(id, _)| *id).collect()
    }

    /// A connection's unique name, once it said `Hello`.
    pub fn unique_name(&self, id: ConnId) -> Option<&str> {
        if id == LOCAL {
            return Some(&self.local_unique);
        }
        self.conns.get(&id)?.unique.as_deref()
    }

    /// The owner of a bus name.
    pub fn owner(&self, name: &str) -> Option<ConnId> {
        self.names.get(name).copied()
    }

    fn fail(&mut self, id: ConnId, why: &str) {
        if let Some(c) = self.conns.get_mut(&id) {
            c.failed.get_or_insert_with(|| why.to_string());
        }
    }

    /// Bytes from a connection: the authentication, then messages.
    pub fn receive(&mut self, id: ConnId, bytes: &[u8]) {
        let Some(c) = self.conns.get_mut(&id) else { return };
        if c.failed.is_some() {
            return;
        }
        let rest = match c.auth.as_mut() {
            Some(auth) => match auth.receive(bytes) {
                Progress::Pending => return,
                Progress::Failed => None,
                Progress::Begun(rest) => {
                    // What the exchange still had to say goes first.
                    let out = auth.take_output();
                    c.out.extend_from_slice(&out);
                    c.auth = None;
                    Some(rest)
                }
            },
            None => Some(bytes.to_vec()),
        };
        let Some(rest) = rest else { return self.fail(id, "authentication failed") };
        let Some(c) = self.conns.get_mut(&id) else { return };
        c.stream.push(&rest);
        loop {
            let Some(c) = self.conns.get_mut(&id) else { return };
            if c.failed.is_some() {
                return;
            }
            match c.stream.next_message() {
                None => return,
                Some(Err(e)) => return self.fail(id, e.0),
                Some(Ok(m)) => self.handle_message(id, m),
            }
        }
    }

    /// A connection went away: its names are released.
    pub fn disconnect(&mut self, id: ConnId) {
        let Some(c) = self.conns.remove(&id) else { return };
        let owned: Vec<String> = self.names.iter().filter(|(_, o)| **o == id).map(|(n, _)| n.clone()).collect();
        for name in owned {
            self.names.remove(&name);
            self.name_owner_changed(&name, c.unique.as_deref().unwrap_or(""), "");
        }
    }

    fn next_serial(&mut self) -> u32 {
        self.serial = self.serial.wrapping_add(1).max(1);
        self.serial
    }

    /// The names a connection is known by.
    fn names_of(&self, id: ConnId) -> Vec<String> {
        self.names.iter().filter(|(_, o)| **o == id).map(|(n, _)| n.clone()).collect()
    }

    fn handle_message(&mut self, id: ConnId, mut m: Message) {
        let unique = self.conns.get(&id).and_then(|c| c.unique.clone());
        let is_hello = m.destination.as_deref() == Some(DRIVER) && m.member.as_deref() == Some("Hello");
        match unique {
            None if !is_hello => return self.fail(id, "the first message must be Hello"),
            Some(_) if is_hello => {
                let e = Message::error(&m, errors::ACCESS_DENIED, "Hello was already called");
                return self.deliver_from_bus(id, e);
            }
            Some(u) => m.sender = Some(u),
            None => {}
        }
        self.route(id, m);
    }

    /// Route a message whose `SENDER` is set.
    fn route(&mut self, from: ConnId, m: Message) {
        match m.destination.clone() {
            Some(d) if d == DRIVER => self.driver(from, m),
            Some(d) => match self.owner(&d) {
                Some(LOCAL) => {
                    if m.kind == MessageType::MethodCall {
                        self.local_inbox.push(m);
                    }
                }
                Some(to) => self.deliver(to, &m),
                None if m.wants_reply() => {
                    let text = format!("The name {d} was not provided by any .service files");
                    let e = Message::error(&m, errors::SERVICE_UNKNOWN, &text);
                    self.deliver_from_bus(from, e);
                }
                None => {}
            },
            None if m.kind == MessageType::Signal => self.broadcast(from, &m),
            None if m.wants_reply() => {
                let e = Message::error(&m, errors::UNKNOWN_METHOD, "a method call needs a destination on a bus");
                self.deliver_from_bus(from, e);
            }
            None => {}
        }
    }

    /// Send `m` (with its own serial) to connection `to`.
    fn deliver(&mut self, to: ConnId, m: &Message) {
        if let Some(c) = self.conns.get_mut(&to) {
            c.out.extend_from_slice(&m.encode(m.serial));
        }
    }

    /// A signal to every connection with a matching rule.
    fn broadcast(&mut self, from: ConnId, m: &Message) {
        let names = self.names_of(from);
        let targets: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(_, c)| c.unique.is_some() && c.matches.iter().any(|r| r.matches(m, &names)))
            .map(|(id, _)| *id)
            .collect();
        for t in targets {
            self.deliver(t, m);
        }
    }

    /// A message from the bus driver to `to` (its serial assigned here).
    fn deliver_from_bus(&mut self, to: ConnId, mut m: Message) {
        m.sender = Some(DRIVER.into());
        m.serial = self.next_serial();
        self.deliver(to, &m);
    }

    /// The driver's `NameOwnerChanged` signal, to whoever matches it.
    fn name_owner_changed(&mut self, name: &str, old: &str, new: &str) {
        let mut s = Message::signal(
            DRIVER_PATH,
            DRIVER,
            "NameOwnerChanged",
            vec![Value::str(name), Value::str(old), Value::str(new)],
        );
        s.sender = Some(DRIVER.into());
        s.serial = self.next_serial();
        let driver = [String::from(DRIVER)];
        let targets: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(_, c)| c.unique.is_some() && c.matches.iter().any(|r| r.matches(&s, &driver)))
            .map(|(id, _)| *id)
            .collect();
        for t in targets {
            self.deliver(t, &s);
        }
    }

    /// `NameAcquired` or `NameLost` to the connection concerned.
    fn name_signal(&mut self, to: ConnId, member: &str, name: &str) {
        if to == LOCAL {
            return;
        }
        let mut s = Message::signal(DRIVER_PATH, DRIVER, member, vec![Value::str(name)]);
        s.destination = self.unique_name(to).map(String::from);
        self.deliver_from_bus(to, s);
    }

    fn reply(&mut self, to: ConnId, call: &Message, body: Vec<Value>) {
        if call.wants_reply() {
            let r = Message::method_return(call, body);
            self.deliver_from_bus(to, r);
        }
    }

    fn error_reply(&mut self, to: ConnId, call: &Message, name: &str, text: &str) {
        if call.wants_reply() {
            let e = Message::error(call, name, text);
            self.deliver_from_bus(to, e);
        }
    }

    /// The bus driver's methods.
    fn driver(&mut self, from: ConnId, m: Message) {
        if m.kind != MessageType::MethodCall {
            return;
        }
        let arg_str = |i: usize| m.body.get(i).and_then(Value::as_str).map(String::from);
        let interface = m.interface.as_deref().unwrap_or(DRIVER);
        let member = m.member.clone().unwrap_or_default();
        match (interface, member.as_str()) {
            (DRIVER, "Hello") => {
                let name = format!(":1.{}", self.next_unique);
                self.next_unique += 1;
                if let Some(c) = self.conns.get_mut(&from) {
                    c.unique = Some(name.clone());
                }
                self.names.insert(name.clone(), from);
                let mut call = m.clone();
                call.sender = Some(name.clone());
                self.reply(from, &call, vec![Value::str(&name)]);
                self.name_signal(from, "NameAcquired", &name);
                self.name_owner_changed(&name, "", &name);
            }
            (DRIVER, "RequestName") => {
                let Some(name) = arg_str(0).filter(|n| !n.starts_with(':') && n.contains('.')) else {
                    return self.error_reply(from, &m, errors::INVALID_ARGS, "not a well-known bus name");
                };
                let code = match self.owner(&name) {
                    Some(o) if o == from => 4, // already the owner
                    Some(_) => 3,              // exists; not queued
                    None => {
                        self.names.insert(name.clone(), from);
                        let unique = self.unique_name(from).unwrap_or("").to_string();
                        self.name_owner_changed(&name, "", &unique);
                        self.name_signal(from, "NameAcquired", &name);
                        1 // primary owner
                    }
                };
                self.reply(from, &m, vec![Value::Uint32(code)]);
            }
            (DRIVER, "ReleaseName") => {
                let name = arg_str(0).unwrap_or_default();
                let code = match self.owner(&name) {
                    Some(o) if o == from && !name.starts_with(':') => {
                        self.names.remove(&name);
                        let unique = self.unique_name(from).unwrap_or("").to_string();
                        self.name_owner_changed(&name, &unique, "");
                        self.name_signal(from, "NameLost", &name);
                        1
                    }
                    Some(_) => 3,
                    None => 2,
                };
                self.reply(from, &m, vec![Value::Uint32(code)]);
            }
            (DRIVER, "GetNameOwner") => {
                let name = arg_str(0).unwrap_or_default();
                match self.owner(&name).and_then(|o| self.unique_name(o)).map(String::from) {
                    Some(u) => self.reply(from, &m, vec![Value::str(&u)]),
                    None if name == DRIVER => self.reply(from, &m, vec![Value::str(DRIVER)]),
                    None => {
                        let text = format!("Could not get owner of name '{name}': no such name");
                        self.error_reply(from, &m, errors::NAME_HAS_NO_OWNER, &text)
                    }
                }
            }
            (DRIVER, "NameHasOwner") => {
                let name = arg_str(0).unwrap_or_default();
                let has = name == DRIVER || self.owner(&name).is_some();
                self.reply(from, &m, vec![Value::Bool(has)]);
            }
            (DRIVER, "ListNames") => {
                let mut names: Vec<&str> = vec![DRIVER];
                names.extend(self.names.keys().map(String::as_str));
                let v = Value::strings(&names);
                self.reply(from, &m, vec![v]);
            }
            (DRIVER, "ListActivatableNames") => self.reply(from, &m, vec![Value::strings(&[DRIVER])]),
            (DRIVER, "StartServiceByName") => {
                let name = arg_str(0).unwrap_or_default();
                if self.owner(&name).is_some() {
                    self.reply(from, &m, vec![Value::Uint32(2)]); // already running
                } else {
                    let text = format!("The name {name} was not provided by any .service files");
                    self.error_reply(from, &m, errors::SERVICE_UNKNOWN, &text);
                }
            }
            (DRIVER, "AddMatch") => match arg_str(0).as_deref().and_then(MatchRule::parse) {
                Some(rule) => {
                    if let Some(c) = self.conns.get_mut(&from) {
                        c.matches.push(rule);
                    }
                    self.reply(from, &m, vec![]);
                }
                None => self.error_reply(from, &m, errors::MATCH_RULE_INVALID, "the match rule is not valid"),
            },
            (DRIVER, "RemoveMatch") => {
                let text = arg_str(0).unwrap_or_default();
                let removed = self.conns.get_mut(&from).is_some_and(|c| {
                    let at = c.matches.iter().position(|r| r.text == text);
                    at.map(|i| c.matches.remove(i)).is_some()
                });
                if removed {
                    self.reply(from, &m, vec![]);
                } else {
                    self.error_reply(from, &m, errors::MATCH_RULE_NOT_FOUND, "no such match rule");
                }
            }
            (DRIVER, "GetId") => {
                let guid = self.guid.clone();
                self.reply(from, &m, vec![Value::str(&guid)]);
            }
            (DRIVER, "GetConnectionUnixUser" | "GetConnectionUnixProcessID" | "GetConnectionCredentials") => {
                self.error_reply(from, &m, errors::NOT_SUPPORTED, "credentials are not tracked on this bus")
            }
            ("org.freedesktop.DBus.Peer", "Ping") => self.reply(from, &m, vec![]),
            ("org.freedesktop.DBus.Peer", "GetMachineId") => {
                let guid = self.guid.clone();
                self.reply(from, &m, vec![Value::str(&guid)]);
            }
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                let xml = "<!DOCTYPE node PUBLIC \"-//freedesktop//DTD D-BUS Object Introspection 1.0//EN\"\n\
                    \"http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd\">\n\
                    <node><interface name=\"org.freedesktop.DBus\"/></node>\n";
                self.reply(from, &m, vec![Value::str(xml)]);
            }
            _ => {
                let text = format!("{interface}.{member} is not offered by this bus");
                self.error_reply(from, &m, errors::UNKNOWN_METHOD, &text);
            }
        }
    }

    // -----------------------------------------------------------------
    // Local services.

    /// Make the bus's own process own `name` (true if it now does).
    pub fn request_local_name(&mut self, name: &str) -> bool {
        match self.owner(name) {
            Some(LOCAL) => true,
            Some(_) => false,
            None => {
                self.names.insert(name.into(), LOCAL);
                let unique = self.local_unique.clone();
                self.name_owner_changed(name, "", &unique);
                true
            }
        }
    }

    /// Method calls to local services since the last call (`SENDER` set).
    pub fn take_local(&mut self) -> Vec<Message> {
        core::mem::take(&mut self.local_inbox)
    }

    /// Send a message from a local service: a reply goes to its
    /// destination, a signal to whoever matches it (or its destination).
    pub fn send_local(&mut self, mut m: Message) {
        m.sender = Some(self.local_unique.clone());
        m.serial = self.next_serial();
        match m.destination.clone() {
            Some(d) => {
                if let Some(to) = self.owner(&d).filter(|&t| t != LOCAL) {
                    self.deliver(to, &m);
                }
            }
            None if m.kind == MessageType::Signal => self.broadcast(LOCAL, &m),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::client_lines;
    use crate::message::Stream;

    const GUID: &str = "00112233445566778899aabbccddeeff";

    /// A client on the bus: authenticated, its replies read back.
    struct Client {
        id: ConnId,
        serial: u32,
        stream: Stream,
    }

    impl Client {
        fn new(bus: &mut Bus) -> Client {
            let id = bus.connect(Some(0));
            let (auth, begin) = client_lines(0);
            bus.receive(id, &auth);
            assert!(bus.take_output(id).starts_with(b"OK 0011"));
            bus.receive(id, &begin);
            let mut c = Client { id, serial: 0, stream: Stream::default() };
            c.call(bus, Message::method_call(Some(DRIVER), DRIVER_PATH, Some(DRIVER), "Hello", vec![]));
            c
        }

        fn call(&mut self, bus: &mut Bus, m: Message) -> u32 {
            self.serial += 1;
            bus.receive(self.id, &m.encode(self.serial));
            self.serial
        }

        fn read(&mut self, bus: &mut Bus) -> Vec<Message> {
            self.stream.push(&bus.take_output(self.id));
            let mut out = Vec::new();
            while let Some(m) = self.stream.next_message() {
                out.push(m.unwrap());
            }
            out
        }
    }

    fn driver_call(member: &str, body: Vec<Value>) -> Message {
        Message::method_call(Some(DRIVER), DRIVER_PATH, Some(DRIVER), member, body)
    }

    #[test]
    fn hello_gives_a_unique_name() {
        let mut bus = Bus::new(GUID);
        let mut a = Client::new(&mut bus);
        let got = a.read(&mut bus);
        assert_eq!(got[0].kind, MessageType::MethodReturn);
        assert_eq!(got[0].reply_serial, Some(1));
        assert_eq!(got[0].body, [Value::str(":1.1")]);
        assert_eq!(got[0].sender.as_deref(), Some(DRIVER));
        assert_eq!(got[1].member.as_deref(), Some("NameAcquired"));
        assert_eq!(bus.unique_name(a.id), Some(":1.1"));
        // Hello again is refused.
        a.call(&mut bus, driver_call("Hello", vec![]));
        assert_eq!(a.read(&mut bus)[0].error_name.as_deref(), Some(errors::ACCESS_DENIED));
    }

    #[test]
    fn anything_before_hello_cuts_the_connection_off() {
        let mut bus = Bus::new(GUID);
        let id = bus.connect(None);
        let (auth, begin) = client_lines(0);
        bus.receive(id, &auth);
        bus.receive(id, &begin);
        bus.receive(id, &driver_call("ListNames", vec![]).encode(1));
        assert!(bus.failure(id).is_some());
    }

    #[test]
    fn names_routing_and_errors() {
        let mut bus = Bus::new(GUID);
        let mut a = Client::new(&mut bus);
        let mut b = Client::new(&mut bus);
        a.read(&mut bus);
        b.read(&mut bus);
        // B owns a name; A calls it; B answers; A gets the answer.
        b.call(&mut bus, driver_call("RequestName", vec![Value::str("org.example.B"), Value::Uint32(0)]));
        let r = b.read(&mut bus);
        assert_eq!(r[0].member.as_deref(), Some("NameAcquired"), "first the signal");
        assert_eq!(r[1].body, [Value::Uint32(1)]);
        a.call(&mut bus, driver_call("RequestName", vec![Value::str("org.example.B"), Value::Uint32(0)]));
        assert_eq!(a.read(&mut bus)[0].body, [Value::Uint32(3)], "taken");
        let serial = a.call(&mut bus, Message::method_call(Some("org.example.B"), "/b", None, "Ping2", vec![]));
        let call = b.read(&mut bus).remove(0);
        assert_eq!((call.serial, call.sender.as_deref()), (serial, Some(":1.1")));
        let reply = Message::method_return(&call, vec![Value::str("pong")]);
        b.call(&mut bus, reply);
        let got = a.read(&mut bus);
        assert_eq!((got[0].reply_serial, &got[0].body[..]), (Some(serial), &[Value::str("pong")][..]));
        assert_eq!(got[0].sender.as_deref(), Some(":1.2"), "the bus sets the sender");
        // An unknown name: ServiceUnknown; GetNameOwner of one: NameHasNoOwner.
        a.call(&mut bus, Message::method_call(Some("org.example.None"), "/", None, "X", vec![]));
        assert_eq!(a.read(&mut bus)[0].error_name.as_deref(), Some(errors::SERVICE_UNKNOWN));
        a.call(&mut bus, driver_call("GetNameOwner", vec![Value::str("org.example.None")]));
        assert_eq!(a.read(&mut bus)[0].error_name.as_deref(), Some(errors::NAME_HAS_NO_OWNER));
        a.call(&mut bus, driver_call("GetNameOwner", vec![Value::str("org.example.B")]));
        assert_eq!(a.read(&mut bus)[0].body, [Value::str(":1.2")]);
        a.call(&mut bus, driver_call("Frobnicate", vec![]));
        assert_eq!(a.read(&mut bus)[0].error_name.as_deref(), Some(errors::UNKNOWN_METHOD));
        // B goes away: A, watching, hears the name lost its owner.
        let rule = "type='signal',sender='org.freedesktop.DBus',member='NameOwnerChanged',arg0='org.example.B'";
        a.call(&mut bus, driver_call("AddMatch", vec![Value::str(rule)]));
        a.read(&mut bus);
        bus.disconnect(b.id);
        let got = a.read(&mut bus);
        assert_eq!(got.len(), 1, "only the rule's name: {got:?}");
        assert_eq!(got[0].body, [Value::str("org.example.B"), Value::str(":1.2"), Value::str("")]);
    }

    #[test]
    fn local_services_and_signals() {
        let mut bus = Bus::new(GUID);
        assert!(bus.request_local_name("org.freedesktop.Notifications"));
        let mut a = Client::new(&mut bus);
        a.read(&mut bus);
        a.call(&mut bus, driver_call("NameHasOwner", vec![Value::str("org.freedesktop.Notifications")]));
        assert_eq!(a.read(&mut bus)[0].body, [Value::Bool(true)]);
        let rule = "type='signal',sender='org.freedesktop.Notifications',interface='org.freedesktop.Notifications'";
        a.call(&mut bus, driver_call("AddMatch", vec![Value::str(rule)]));
        a.read(&mut bus);
        let serial = a.call(
            &mut bus,
            Message::method_call(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                None,
                "X",
                vec![],
            ),
        );
        let calls = bus.take_local();
        assert_eq!((calls.len(), calls[0].serial, calls[0].sender.as_deref()), (1, serial, Some(":1.1")));
        bus.send_local(Message::method_return(&calls[0], vec![Value::Uint32(7)]));
        // A signal from the service reaches the matching client, from the
        // service's unique name.
        bus.send_local(Message::signal(
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
            "NotificationClosed",
            vec![Value::Uint32(7), Value::Uint32(1)],
        ));
        let got = a.read(&mut bus);
        assert_eq!(got[0].body, [Value::Uint32(7)]);
        assert_eq!((got[1].member.as_deref(), got[1].sender.as_deref()), (Some("NotificationClosed"), Some(":1.0")));
    }

    #[test]
    fn match_rules() {
        let r = MatchRule::parse("type='signal',interface='a.b',path_namespace='/x',arg0='it'\\''s'").unwrap();
        assert_eq!(r.kind, Some(MessageType::Signal));
        assert_eq!(r.args, [(0, String::from("it's"))]);
        let mut m = Message::signal("/x/y", "a.b", "M", vec![Value::str("it's")]);
        assert!(r.matches(&m, &[]));
        m.path = Some("/xy".into());
        assert!(!r.matches(&m, &[]), "not inside the namespace");
        assert!(MatchRule::parse("type='nope'").is_none());
        assert!(MatchRule::parse("bogus='1'").is_none());
        assert!(MatchRule::parse("member='open").is_none());
        assert_eq!(MatchRule::parse("").unwrap().kind, None);
    }
}
