#!/usr/bin/env python3
"""Migrate physical Event streams to typed retained-credit ingress, not control channels.

Run from the checkout root. Semantic visitor/actor changes are separate commits.
"""
from pathlib import Path

root = Path("crates/horizon-client/src")
net = root / "net.rs"
source = net.read_text()
source = source.replace("Receiver<Event>", "ingress::Consumer<Event>")
source = source.replace("Sender<Event>", "ingress::Producer<Event>")
source = source.replace("        mpsc::{Receiver, Sender, channel},\n", "")
if "pub mod ingress;" not in source:
    source = source.replace("pub(crate) mod queued_writer;", "pub(crate) mod queued_writer;\npub mod ingress;\n#[cfg(test)]\nuse std::sync::mpsc::channel;")
for lhs in ("(send, events)", "(_send, events)", "(send, _events)"):
    source = source.replace(f"let {lhs} = channel();", f"let {lhs} = event_channel();")
source = source.replace("payload_events: channel().0,", "payload_events: event_channel().0,")
net.write_text(source)

path = root / "raw_payloads/endpoint.rs"
source = path.read_text().replace("    mpsc::Sender,\n", "")
source = source.replace("Sender<Event>", "crate::net::ingress::Producer<Event>")
path.write_text(source)

path = root / "lan/mod.rs"
source = path.read_text().replace("Sender<NetEvent>", "crate::net::ingress::Producer<NetEvent>")
source = source.replace("Receiver<NetEvent>", "crate::net::ingress::Consumer<NetEvent>")
source = source.replace("mpsc::{Receiver, Sender, channel}", "mpsc::{Receiver, channel}")
source = source.replace("let (tx, events) = channel();", "let (tx, events) = event_channel();")
path.write_text(source)

path = root / "lan/config_payloads.rs"
source = path.read_text().replace("use std::sync::{Arc, mpsc::Sender};", "use std::sync::Arc;")
source = source.replace("Sender<super::NetEvent>", "crate::net::ingress::Producer<super::NetEvent>")
path.write_text(source)
