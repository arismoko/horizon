//! Concrete payload authoring; frozen lookup storage stays in the vocabulary.
use super::*;

#[cfg(test)]
#[path = "payloads/tests.rs"]
mod tests;

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
}
