//! The messages of COM across processes (ADR 0220): what `ole32` and the activation process
//! `comlaunch.exe` send each other over named pipes.
//!
//! Every connection carries one request and one reply, each a frame: a 32-bit little-endian
//! length, then the payload. Payloads start with a kind byte; numbers are little-endian, strings
//! UTF-16 with a 32-bit length in code units, GUIDs their 16 bytes in memory order. This is Poly's
//! own format (nothing on the wire is observable to programs).

use crate::guid::Guid;
use crate::hresult::Hresult;
use alloc::string::String;
use alloc::vec::Vec;

/// The pipe of the activation process.
pub const ACTIVATION_PIPE: &str = r"\\.\pipe\poly-com-activation";

/// The longest payload accepted.
pub const MAX_FRAME: usize = 16 * 1024 * 1024;

/// How the activation process starts a class's server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// A command line (`LocalServer32`, with `-Embedding` already added).
    Command(String),
    /// A service (`AppID\LocalService`) and its arguments (`ServiceParameters`).
    Service { name: String, args: Vec<String> },
}

/// A reference to an exported object: the exporter's pipe and the object's id there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjRef {
    pub endpoint: String,
    pub oid: u64,
}

/// An operation on an exported object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    /// `IUnknown::QueryInterface`: whether the object has the interface (the proxy answers it).
    QueryInterface(Guid),
    /// Give back `n` references the caller's proxy held.
    Release(u32),
    /// `IClassFactory::CreateInstance(NULL, iid)`: the new object comes back as a reference.
    CreateInstance(Guid),
    /// `IClassFactory::LockServer(lock)`.
    LockServer(bool),
    /// A method of interface `iid` (B2b): the vtable slot and the marshalled arguments, which the
    /// interface's stub reads; the reply carries the marshalled results.
    Invoke { iid: Guid, method: u32, data: Vec<u8> },
    /// Add `n` references for a proxy that another party marshalled (B2b); with `token` not 0,
    /// only while the table registration `token` still exists (`CO_E_OBJNOTREG` otherwise).
    AddRef { n: u32, token: u64 },
}

/// A request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// A server publishes a class object: process id, its cookie, the factory's reference, single use.
    Register { clsid: Guid, pid: u32, cookie: u32, factory: ObjRef, single: bool },
    /// A server withdraws one.
    Revoke { pid: u32, cookie: u32 },
    /// A client wants the class object of `clsid` (and how to start a server for it, if it knows).
    Activate { clsid: Guid, plan: Option<Plan> },
    /// A call on an exported object.
    Call { oid: u64, op: Op },
}

/// A reply: a result and possibly an object reference (the class object of an activation, the
/// object a `CreateInstance` made). For an activation that served a single-use registration, the
/// server is told with `single_used` set to the cookie (0 otherwise).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    pub hr: Hresult,
    pub obj: Option<ObjRef>,
    /// The marshalled results of an `Invoke`.
    pub data: Vec<u8>,
}

/// A message that does not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Malformed;

struct W(Vec<u8>);

impl W {
    fn u8(&mut self, v: u8) {
        self.0.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.0.extend_from_slice(&v.to_le_bytes());
    }
    fn guid(&mut self, g: &Guid) {
        self.u32(g.data1);
        self.0.extend_from_slice(&g.data2.to_le_bytes());
        self.0.extend_from_slice(&g.data3.to_le_bytes());
        self.0.extend_from_slice(&g.data4);
    }
    fn str(&mut self, s: &str) {
        let units: Vec<u16> = s.encode_utf16().collect();
        self.u32(units.len() as u32);
        for u in units {
            self.0.extend_from_slice(&u.to_le_bytes());
        }
    }
    fn bytes(&mut self, b: &[u8]) {
        self.u32(b.len() as u32);
        self.0.extend_from_slice(b);
    }
    fn objref(&mut self, o: &ObjRef) {
        self.str(&o.endpoint);
        self.u64(o.oid);
    }
}

struct R<'a>(&'a [u8]);

impl R<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], Malformed> {
        if self.0.len() < n {
            return Err(Malformed);
        }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn u8(&mut self) -> Result<u8, Malformed> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32, Malformed> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().map_err(|_| Malformed)?))
    }
    fn u64(&mut self) -> Result<u64, Malformed> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().map_err(|_| Malformed)?))
    }
    fn guid(&mut self) -> Result<Guid, Malformed> {
        let data1 = self.u32()?;
        let b = self.take(4)?;
        let data2 = u16::from_le_bytes([b[0], b[1]]);
        let data3 = u16::from_le_bytes([b[2], b[3]]);
        let data4 = self.take(8)?.try_into().map_err(|_| Malformed)?;
        Ok(Guid { data1, data2, data3, data4 })
    }
    fn str(&mut self) -> Result<String, Malformed> {
        let n = self.u32()? as usize;
        if n > MAX_FRAME {
            return Err(Malformed);
        }
        let b = self.take(n * 2)?;
        let units: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&units).map_err(|_| Malformed)
    }
    fn bytes(&mut self) -> Result<Vec<u8>, Malformed> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn objref(&mut self) -> Result<ObjRef, Malformed> {
        Ok(ObjRef { endpoint: self.str()?, oid: self.u64()? })
    }
    fn end(&self) -> Result<(), Malformed> {
        if self.0.is_empty() { Ok(()) } else { Err(Malformed) }
    }
}

impl Request {
    /// The payload (without the frame's length).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        match self {
            Request::Register { clsid, pid, cookie, factory, single } => {
                w.u8(1);
                w.guid(clsid);
                w.u32(*pid);
                w.u32(*cookie);
                w.objref(factory);
                w.u8(u8::from(*single));
            }
            Request::Revoke { pid, cookie } => {
                w.u8(2);
                w.u32(*pid);
                w.u32(*cookie);
            }
            Request::Activate { clsid, plan } => {
                w.u8(3);
                w.guid(clsid);
                match plan {
                    None => w.u8(0),
                    Some(Plan::Command(c)) => {
                        w.u8(1);
                        w.str(c);
                    }
                    Some(Plan::Service { name, args }) => {
                        w.u8(2);
                        w.str(name);
                        w.u32(args.len() as u32);
                        for a in args {
                            w.str(a);
                        }
                    }
                }
            }
            Request::Call { oid, op } => {
                w.u8(4);
                w.u64(*oid);
                match op {
                    Op::QueryInterface(iid) => {
                        w.u8(1);
                        w.guid(iid);
                    }
                    Op::Release(n) => {
                        w.u8(2);
                        w.u32(*n);
                    }
                    Op::CreateInstance(iid) => {
                        w.u8(3);
                        w.guid(iid);
                    }
                    Op::LockServer(l) => {
                        w.u8(4);
                        w.u8(u8::from(*l));
                    }
                    Op::Invoke { iid, method, data } => {
                        w.u8(5);
                        w.guid(iid);
                        w.u32(*method);
                        w.bytes(data);
                    }
                    Op::AddRef { n, token } => {
                        w.u8(6);
                        w.u32(*n);
                        w.u64(*token);
                    }
                }
            }
        }
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Request, Malformed> {
        let mut r = R(b);
        let req = match r.u8()? {
            1 => Request::Register {
                clsid: r.guid()?,
                pid: r.u32()?,
                cookie: r.u32()?,
                factory: r.objref()?,
                single: r.u8()? != 0,
            },
            2 => Request::Revoke { pid: r.u32()?, cookie: r.u32()? },
            3 => {
                let clsid = r.guid()?;
                let plan = match r.u8()? {
                    0 => None,
                    1 => Some(Plan::Command(r.str()?)),
                    2 => {
                        let name = r.str()?;
                        let n = r.u32()? as usize;
                        if n > 256 {
                            return Err(Malformed);
                        }
                        let mut args = Vec::with_capacity(n);
                        for _ in 0..n {
                            args.push(r.str()?);
                        }
                        Some(Plan::Service { name, args })
                    }
                    _ => return Err(Malformed),
                };
                Request::Activate { clsid, plan }
            }
            4 => {
                let oid = r.u64()?;
                let op = match r.u8()? {
                    1 => Op::QueryInterface(r.guid()?),
                    2 => Op::Release(r.u32()?),
                    3 => Op::CreateInstance(r.guid()?),
                    4 => Op::LockServer(r.u8()? != 0),
                    5 => Op::Invoke { iid: r.guid()?, method: r.u32()?, data: r.bytes()? },
                    6 => Op::AddRef { n: r.u32()?, token: r.u64()? },
                    _ => return Err(Malformed),
                };
                Request::Call { oid, op }
            }
            _ => return Err(Malformed),
        };
        r.end()?;
        Ok(req)
    }
}

impl Reply {
    pub fn hr(hr: Hresult) -> Reply {
        Reply { hr, obj: None, data: Vec::new() }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = W(Vec::new());
        w.u32(self.hr as u32);
        match &self.obj {
            None => w.u8(0),
            Some(o) => {
                w.u8(1);
                w.objref(o);
            }
        }
        w.bytes(&self.data);
        w.0
    }

    pub fn decode(b: &[u8]) -> Result<Reply, Malformed> {
        let mut r = R(b);
        let hr = r.u32()? as Hresult;
        let obj = match r.u8()? {
            0 => None,
            1 => Some(r.objref()?),
            _ => return Err(Malformed),
        };
        let data = r.bytes()?;
        r.end()?;
        Ok(Reply { hr, obj, data })
    }
}

/// A frame: the payload's length, then the payload.
pub fn frame(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 4);
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

/// The payload length a frame's first four bytes announce, if acceptable.
pub fn frame_len(head: [u8; 4]) -> Option<usize> {
    let n = u32::from_le_bytes(head) as usize;
    (n <= MAX_FRAME).then_some(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    const G: Guid = Guid { data1: 0x1234_5678, data2: 0x9ABC, data3: 0xDEF0, data4: [1, 2, 3, 4, 5, 6, 7, 8] };

    fn round(r: Request) {
        assert_eq!(Request::decode(&r.encode()), Ok(r));
    }

    #[test]
    fn requests_round_trip() {
        let o = ObjRef { endpoint: r"\\.\pipe\poly-com-ox-5-ab".to_string(), oid: 0x8000_0000_0000_1001 };
        round(Request::Register { clsid: G, pid: 5, cookie: 0x1001, factory: o.clone(), single: true });
        round(Request::Revoke { pid: 5, cookie: 7 });
        round(Request::Activate { clsid: G, plan: None });
        round(Request::Activate { clsid: G, plan: Some(Plan::Command("\"C:\\a b\\s.exe\" -Embedding".to_string())) });
        round(Request::Activate {
            clsid: G,
            plan: Some(Plan::Service { name: "Svc".to_string(), args: vec!["--one".to_string(), "ü".to_string()] }),
        });
        for op in [
            Op::QueryInterface(G),
            Op::Release(3),
            Op::CreateInstance(G),
            Op::LockServer(true),
            Op::Invoke { iid: G, method: 7, data: vec![1, 2, 3] },
            Op::AddRef { n: 1, token: 9 },
        ] {
            round(Request::Call { oid: 9, op });
        }
        let rep = Reply { hr: -2_147_221_164, obj: Some(o), data: vec![9, 8] };
        assert_eq!(Reply::decode(&rep.encode()), Ok(rep));
        assert_eq!(Reply::decode(&Reply::hr(0).encode()), Ok(Reply::hr(0)));
    }

    #[test]
    fn junk_is_refused() {
        assert_eq!(Request::decode(&[]), Err(Malformed));
        assert_eq!(Request::decode(&[9]), Err(Malformed));
        let mut b = Request::Revoke { pid: 1, cookie: 2 }.encode();
        b.push(0);
        assert_eq!(Request::decode(&b), Err(Malformed));
        assert_eq!(frame_len((MAX_FRAME as u32 + 1).to_le_bytes()), None);
        assert_eq!(&frame(&[7, 8])[..], &[2, 0, 0, 0, 7, 8]);
    }

    #[test]
    fn guid_bytes_are_memory_order() {
        let mut w = W(Vec::new());
        w.guid(&G);
        assert_eq!(w.0, [0x78, 0x56, 0x34, 0x12, 0xBC, 0x9A, 0xF0, 0xDE, 1, 2, 3, 4, 5, 6, 7, 8]);
    }
}
