//! The records the writer thread takes: a tag, or a chunk still to encode on that thread.
use super::*;

/// A record waiting for the writer thread.
#[derive(Clone)]
pub(super) enum Record {
    Disk(Arc<chunk_disk::DiskRecord>),
    EditedChunk(Arc<EditedChunk>),
    Tag(Arc<NamedTag>),
    /// A chunk to encode on the writer thread.
    /// The old chunk's calculated blending data is taken when the save is
    /// requested ([`SaveDir::calculated_blending`]): a snapshot of that save.
    Chunk(Arc<Chunk>, Arc<Encoding>, Option<Arc<BlendingData>>),
    Proto(Arc<SavedProto>, Arc<Encoding>),
    /// A chunk's points of interest, made on the writer thread against the
    /// saved ones (`resolve_poi`).
    Poi(
        Arc<Chunk>,
        Arc<SaveTables>,
        Option<Arc<crate::mobs::villager::ResolvedProfessions>>,
    ),
    Delete,
    #[cfg(test)]
    TestEncode(Arc<dyn Fn(bool) -> Option<Arc<NamedTag>> + Send + Sync>),
}
/// The pending reader only waits for this record's final payload selection.
pub(super) struct EditedChunk {
    original: Record,
    admitted: OnceLock<bool>,
}

/// What encoding a chunk needs, captured when it is queued.
pub(super) struct Encoding {
    pub(super) items: Arc<crate::items::Items>,
    pub(super) registry: &'static BlockRegistry,
    pub(super) biomes: Arc<Biomes>,
    pub(super) tables: Option<Arc<SaveTables>>,
    pub(super) shape: LevelShape,
}
impl Record {
    pub(super) fn track_block_entity_edits(self) -> Self {
        if self.has_block_entity_edits() {
            Self::EditedChunk(Arc::new(EditedChunk {
                original: self,
                admitted: OnceLock::new(),
            }))
        } else {
            self
        }
    }

    fn has_block_entity_edits(&self) -> bool {
        match self {
            Self::Chunk(chunk, ..) => chunk
                .saved
                .as_ref()
                .is_some_and(|x| x.block_entity_save_edits.is_some()),
            Self::Disk(record) => record.original.has_block_entity_edits(),
            Self::EditedChunk(_) => true,
            _ => false,
        }
    }

    pub(super) fn accept_block_entity_edits(&self) {
        self.resolve_block_entity_edits(true);
    }

    pub(super) fn resolve_block_entity_edits(&self, admitted: bool) {
        if let Self::EditedChunk(record) = self {
            let _ = record.admitted.set(admitted);
        }
    }

    pub(super) fn source(&self) -> &Self {
        match self {
            Self::EditedChunk(record) => &record.original,
            _ => self,
        }
    }

    pub(super) fn disk_job(&self) -> Option<&chunk_disk::DiskRecord> {
        match self.source() {
            Self::Disk(record) => Some(record),
            _ => None,
        }
    }

    /// Re-encode only the stored block-entity payloads from this same save snapshot.
    pub(super) fn without_block_entity_edits(&self) -> Option<Arc<NamedTag>> {
        self.stored_block_entities(None)
    }

    pub(super) fn stored_block_entities(
        &self,
        attachments: Option<&Compound>,
    ) -> Option<Arc<NamedTag>> {
        match self {
            Self::EditedChunk(record) => record.original.stored_block_entities(attachments),
            Self::Disk(record) => record.without_block_entity_edits(),
            Self::Chunk(chunk, e, blending) => {
                chunk.saved.as_ref()?.block_entity_save_edits.as_ref()?;
                Self::chunk_tag(
                    chunk,
                    e,
                    blending.as_deref(),
                    ChunkEncodingEdits {
                        block_entities: false,
                        attachments,
                    },
                )
                .ok()
            }
            #[cfg(test)]
            Self::TestEncode(encode) => encode(false),
            _ => None,
        }
    }

    pub(super) fn tag(&self) -> Option<Arc<NamedTag>> {
        if let Self::EditedChunk(record) = self {
            return if *record.admitted.wait() {
                record.original.tag_for_write().ok().flatten()
            } else {
                record.original.without_block_entity_edits()
            };
        }
        self.encode_tag(false, None).ok().flatten()
    }

    /// Metadata and far terrain do not consume a block entity's saved payload.
    pub(super) fn tag_for_metadata(&self) -> Option<Arc<NamedTag>> {
        if let Self::EditedChunk(record) = self {
            return record.original.without_block_entity_edits();
        }
        self.tag()
    }

    pub(super) fn tag_for_write(&self) -> std::io::Result<Option<Arc<NamedTag>>> {
        self.encode_tag(true, None)
    }

    pub(super) fn encode_tag(
        &self,
        edits: bool,
        attachments: Option<&Compound>,
    ) -> std::io::Result<Option<Arc<NamedTag>>> {
        match self {
            Record::EditedChunk(record) => record.original.encode_tag(edits, attachments),
            Record::Disk(record) => Ok(record.read()),
            Record::Tag(t) => Ok(Some(t.clone())),
            Record::Chunk(c, e, blending) => Ok(Some(Self::chunk_tag(
                c,
                e,
                blending.as_deref(),
                ChunkEncodingEdits {
                    block_entities: edits,
                    attachments,
                },
            )?)),
            Record::Proto(p, e) => Ok(Some(Arc::new(
                encode_proto_with_attachments(
                    p,
                    e.registry,
                    &e.biomes,
                    e.tables.as_deref(),
                    e.shape,
                    &e.items,
                    attachments,
                )
                .map_err(std::io::Error::other)?,
            ))),
            Record::Poi(c, t, professions) => {
                Ok(encode_poi(c, t, professions.as_deref()).map(Arc::new))
            }
            Record::Delete => Ok(None),
            #[cfg(test)]
            Record::TestEncode(encode) => Ok(encode(edits)),
        }
    }

    fn chunk_tag(
        c: &Chunk,
        e: &Encoding,
        blending: Option<&BlendingData>,
        edits: ChunkEncodingEdits<'_>,
    ) -> std::io::Result<Arc<NamedTag>> {
        let mut tag = encode_full(
            c,
            e.registry,
            &e.biomes,
            e.tables.as_deref(),
            e.shape,
            &e.items,
            edits,
        )
        .map_err(std::io::Error::other)?;
        if let Some(data) = blending {
            put_calculated_blending(&mut tag, data);
        }
        Ok(Arc::new(tag))
    }
}
