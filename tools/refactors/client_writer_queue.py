#!/usr/bin/env python3
"""Migrate the client's shared writer type to its bounded asynchronous owner.

Run from the repository root. The writer implementation and production startup
error handling are intentionally separate hand-written changes.
"""
from pathlib import Path

path = Path("crates/horizon-client/src/net.rs")
source = path.read_text()
source = source.replace("ConnReader, ConnWriter, Connection", "ConnReader, Connection")
source = source.replace("Mutex<Option<ConnWriter>>", "Mutex<Option<QueuedWriter>>")
source = source.replace("ConnWriter::server_version", "QueuedWriter::server_version")
source = source.replace("ConnWriter::legacy_secure_chat_enforcement", "QueuedWriter::legacy_secure_chat_enforcement")
# Tests construct local connections directly. Their writer is now owned by the
# same worker as production, rather than an unbounded test-only shortcut.
source = source.replace("Mutex::new(Some(writer))", "Mutex::new(Some(QueuedWriter::new(writer).unwrap()))")
path.write_text(source)
