//! Concrete payload authoring; frozen lookup storage stays in the vocabulary.
use super::*;

#[cfg(test)]
#[path = "payloads/tests.rs"]
mod tests;

/// Namespaces the engine owns: `minecraft:` (`brand`, `register`, ...) and every
/// `horizon:` channel. A raw declaration cannot claim them.
const RESERVED_RAW_NAMESPACES: [&str; 2] = ["minecraft", "horizon"];

fn indices(protocol: PayloadProtocol) -> &'static [usize] {
    match protocol {
        PayloadProtocol::Configuration => &[0],
        PayloadProtocol::Play => &[1],
        PayloadProtocol::Common => &[0, 1],
    }
}

impl PayloadsRegistrar for PartRegistrar {
    fn add_payload(
        &mut self,
        mut declaration: PayloadDeclaration,
        server: Option<Arc<dyn IPayloadHandler>>,
        client: Option<Arc<dyn IPayloadHandler>>,
    ) {
        let refused = (|| {
            if !declaration.id.0.contains(':') {
                return Err("payload ids must be explicitly namespaced".to_owned());
            }
            declaration.id.0 = self.owned_declaration_id(&declaration.id.0)?;
            if declaration.version.trim().is_empty() {
                return Err("payload version must not be blank".to_owned());
            }
            if declaration.direction != PayloadDirection::ToClient && server.is_none() {
                return Err("serverbound payload requires a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToClient && server.is_some() {
                return Err("clientbound-only payload has a server handler".to_owned());
            }
            if declaration.direction == PayloadDirection::ToServer && client.is_some() {
                return Err("serverbound-only payload has a client handler".to_owned());
            }
            if let Some(payloads) = &self.parts.payloads {
                if let Some(raw) = payloads.raw.get(&declaration.id.0) {
                    let owners: Vec<_> = raw.iter().map(|owner| owner.owner).collect();
                    return Err(format!(
                        "it collides with the raw channel declared by {}",
                        owners.join(", ")
                    ));
                }
                for &index in indices(declaration.protocol) {
                    if payloads.phases[index].contains_key(&declaration.id.0) {
                        return Err(format!(
                            "duplicate payload registration for {} in protocol {index}",
                            declaration.id.0
                        ));
                    }
                }
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} registered payload {}: {reason}",
                self.mod_id, declaration.id.0
            ));
            return;
        }
        let payloads = Arc::make_mut(
            self.parts
                .payloads
                .get_or_insert_with(|| Arc::new(Payloads::default())),
        );
        for &index in indices(declaration.protocol) {
            let mut normalized = declaration.clone();
            normalized.protocol = if index == 0 {
                PayloadProtocol::Configuration
            } else {
                PayloadProtocol::Play
            };
            payloads.phases[index].insert(
                declaration.id.0.clone(),
                Registered {
                    declaration: normalized,
                    owner: self.mod_id,
                    owner_index: self.mod_index,
                    server: server.clone(),
                    client: client.clone(),
                    client_thread: declaration.thread,
                },
            );
        }
    }

    fn add_client_payload_handler(
        &mut self,
        id: &PayloadId,
        thread: HandlerThread,
        handler: Arc<dyn IPayloadHandler>,
    ) {
        let Some(payloads) = self.parts.payloads.as_mut() else {
            self.refused.push(format!(
                "{} supplied a client handler for unknown payload {}",
                self.mod_id, id.0
            ));
            return;
        };
        let mut found = false;
        for phase in &payloads.phases {
            if let Some(entry) = phase.get(&id.0) {
                if entry.owner != self.mod_id {
                    self.refused.push(format!(
                        "{} cannot attach a client handler to {} owned by {}",
                        self.mod_id, id.0, entry.owner
                    ));
                    return;
                }
                if entry.declaration.direction != PayloadDirection::ToServer {
                    found = true;
                    if entry.client.is_some() {
                        self.refused.push(format!(
                            "{} registered a second client payload handler for {}",
                            self.mod_id, id.0
                        ));
                        return;
                    }
                }
            }
        }
        if !found {
            self.refused.push(format!(
                "{} supplied a client handler for unknown or serverbound-only payload {}",
                self.mod_id, id.0
            ));
            return;
        }
        for phase in &mut Arc::make_mut(payloads).phases {
            if let Some(entry) = phase.get_mut(&id.0)
                && entry.declaration.direction != PayloadDirection::ToServer
            {
                entry.client = Some(handler.clone());
                entry.client_thread = thread;
            }
        }
    }

    fn add_raw_payload(&mut self, id: PayloadId, server: Option<Arc<dyn IPayloadHandler>>) {
        let refused = (|| {
            let parsed = Identifier::parse_namespaced(&id.0)
                .ok_or_else(|| "a raw channel needs an exact namespaced id".to_owned())?;
            if RESERVED_RAW_NAMESPACES.contains(&parsed.namespace()) {
                return Err(format!(
                    "the {:?} namespace belongs to the engine",
                    parsed.namespace()
                ));
            }
            let Some(payloads) = &self.parts.payloads else {
                return Ok(());
            };
            for phase in &payloads.phases {
                if let Some(typed) = phase.get(&id.0) {
                    return Err(format!(
                        "it collides with the typed payload registered by {}",
                        typed.owner
                    ));
                }
            }
            if payloads
                .raw
                .get(&id.0)
                .is_some_and(|owners| owners.iter().any(|owner| owner.owner == self.mod_id))
            {
                return Err("it is already declared by this mod".to_owned());
            }
            Ok(())
        })();
        if let Err(reason) = refused {
            self.refused.push(format!(
                "{} declared raw channel {}: {reason}",
                self.mod_id, id.0
            ));
            return;
        }
        let payloads = Arc::make_mut(
            self.parts
                .payloads
                .get_or_insert_with(|| Arc::new(Payloads::default())),
        );
        payloads.raw.entry(id.0).or_default().push(RawRegistered {
            owner: self.mod_id,
            owner_index: self.mod_index,
            server,
        });
    }
}
