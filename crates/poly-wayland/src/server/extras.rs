//! Two small protocols toolkits use when they are there:
//!
//! - `wp_fractional_scale_v1` (fractional-scale-v1.xml): a surface's
//!   preferred scale in 120ths. Poly's outputs have an integer scale, so it
//!   is that scale times 120; a client that uses it draws at the output's
//!   resolution through `wp_viewport`, which Poly shows pixel for pixel.
//! - `xdg_activation_v1` (xdg-activation-v1.xml): a client asks for a
//!   token, hands it to another (an app it starts, through
//!   `XDG_ACTIVATION_TOKEN`), which activates its toplevel with it. Tokens
//!   are issued to anyone (Poly does not check the serial yet), used once,
//!   and the 32 newest are kept.

use super::{ClientId, Data, Res, Server, error};
use crate::protocol::{
    wp_fractional_scale_manager_v1, wp_fractional_scale_v1, xdg_activation_token_v1, xdg_activation_v1,
};
use crate::wire::ObjectId;
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;

/// How many unused activation tokens are kept.
const TOKENS: usize = 32;

/// An `xdg_activation_token_v1` being built.
#[derive(Debug, Default)]
pub(crate) struct Token {
    pub app_id: Option<String>,
    pub surface: Option<ObjectId>,
    pub committed: bool,
}

impl Server {
    pub(super) fn fractional_manager_request(
        &mut self,
        c: ClientId,
        _manager: ObjectId,
        req: wp_fractional_scale_manager_v1::Request,
    ) -> Res {
        let wp_fractional_scale_manager_v1::Request::GetFractionalScale { id, surface } = req else { return Ok(()) };
        let Some(s) = self.surface(c, surface) else { return Ok(()) };
        if s.fractional.is_some() {
            let code = wp_fractional_scale_manager_v1::Error::FRACTIONAL_SCALE_EXISTS.0;
            return Err(error(_manager, code, format!("surface {surface} already has a fractional scale")));
        }
        s.fractional = Some(id);
        let scale = self.output.scale * 120;
        let client = self.client(c);
        client.insert(id, &wp_fractional_scale_v1::INTERFACE, 1, Data::Fractional(Some(surface)));
        client.send(id, wp_fractional_scale_v1::Event::PreferredScale { scale });
        Ok(())
    }

    pub(super) fn fractional_request(
        &mut self,
        c: ClientId,
        id: ObjectId,
        _req: wp_fractional_scale_v1::Request,
    ) -> Res {
        // destroy
        if let Some(Data::Fractional(Some(surface))) = self.client(c).data(id) {
            let surface = *surface;
            if let Some(s) = self.surface(c, surface) {
                s.fractional = None;
            }
        }
        Ok(())
    }

    /// Every fractional scale object hears the output's scale.
    pub(super) fn fractional_scale_changed(&mut self) {
        let scale = self.output.scale * 120;
        for client in self.clients.values_mut() {
            for id in client.ids_where(|d| matches!(d, Data::Fractional(Some(_)))) {
                client.send(id, wp_fractional_scale_v1::Event::PreferredScale { scale });
            }
        }
    }

    pub(super) fn activation_request(&mut self, c: ClientId, _id: ObjectId, req: xdg_activation_v1::Request) -> Res {
        match req {
            xdg_activation_v1::Request::GetActivationToken { id } => {
                let iface = &xdg_activation_token_v1::INTERFACE;
                self.client(c).insert(id, iface, 1, Data::ActivationToken(Box::default()));
            }
            xdg_activation_v1::Request::Activate { token, surface } => {
                let Some(i) = self.tokens.iter().position(|t| *t == token) else { return Ok(()) };
                self.tokens.remove(i);
                let main = self.main_surface(c, surface);
                let toplevel = self
                    .surface_ref(c, main)
                    .and_then(|s| s.xdg)
                    .and_then(|x| match self.clients.get(&c)?.data(x) {
                        Some(Data::XdgSurface(xs)) => xs.role,
                        _ => None,
                    })
                    .filter(|r| matches!(self.clients.get(&c).and_then(|cl| cl.data(*r)), Some(Data::Toplevel(_))));
                if let Some(tl) = toplevel {
                    self.activate(c, tl, true);
                }
            }
            xdg_activation_v1::Request::Destroy => {}
        }
        Ok(())
    }

    pub(super) fn activation_token_request(
        &mut self,
        c: ClientId,
        id: ObjectId,
        req: xdg_activation_token_v1::Request,
    ) -> Res {
        let Some(Data::ActivationToken(t)) = self.client(c).data_mut(id) else { return Ok(()) };
        let used = || error(id, xdg_activation_token_v1::Error::ALREADY_USED.0, "the token was committed already");
        match req {
            xdg_activation_token_v1::Request::SetSerial { .. } if t.committed => return Err(used()),
            xdg_activation_token_v1::Request::SetAppId { .. } if t.committed => return Err(used()),
            xdg_activation_token_v1::Request::SetSurface { .. } if t.committed => return Err(used()),
            xdg_activation_token_v1::Request::Commit if t.committed => return Err(used()),
            xdg_activation_token_v1::Request::SetAppId { app_id } => t.app_id = Some(app_id),
            xdg_activation_token_v1::Request::SetSurface { surface } => t.surface = Some(surface),
            xdg_activation_token_v1::Request::Commit => {
                t.committed = true;
                self.token_serial += 1;
                let token = format!("poly-activation-{}-{}", c.0, self.token_serial);
                if self.tokens.len() == TOKENS {
                    self.tokens.remove(0);
                }
                self.tokens.push(token.clone());
                self.client(c).send(id, xdg_activation_token_v1::Event::Done { token });
            }
            xdg_activation_token_v1::Request::SetSerial { .. } | xdg_activation_token_v1::Request::Destroy => {}
        }
        Ok(())
    }

    /// A fresh activation token, as if a client asked for one (the shell
    /// hands it to an app it starts as `XDG_ACTIVATION_TOKEN`).
    pub fn activation_token(&mut self) -> String {
        self.token_serial += 1;
        let token = format!("poly-activation-0-{}", self.token_serial);
        if self.tokens.len() == TOKENS {
            self.tokens.remove(0);
        }
        self.tokens.push(token.clone());
        token
    }
}
