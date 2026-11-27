use super::*;

/// One dimension folder (`dimensions/minecraft/<name>`). Region files are
/// read lazily by worker threads.
pub struct SaveDir {
    pub root: PathBuf,
    pub(super) save_reporting: writes::SaveReporting,
    /// The loaded catalog for stack encoding; absent for metadata-only readers.
    items: Option<Arc<crate::items::Items>>,
    pub(super) regions: Arc<RegionCache>,
    pub(super) entity_regions: Arc<RegionCache>,
    /// `PersistentEntitySectionManager` leaves a failed read pending for the life of the level.
    pub(super) failed_entity_reads: Mutex<HashSet<(i32, i32)>>,
    pub(super) poi_regions: Arc<RegionCache>,
    pub(super) pending: Arc<PendingWrites>,
    pub(super) sequence: AtomicU64,
    pub(super) disk: OnceLock<Arc<chunk_disk::DiskRuntime>>,
    pub(super) writer: Mutex<Option<Writer>>,
    pub(super) outstanding: Arc<Outstanding>,
    /// Set by `Persistent` before the first load.
    pub tables: OnceLock<Arc<SaveTables>>,
    pub(super) professions:
        std::sync::RwLock<Option<Arc<crate::mobs::villager::ResolvedProfessions>>>,
    /// A server-owned world save, not a remote cache: chunk-disk hooks need its worker context.
    pub(super) server_owned: bool,
    /// The dimension's skylight and height, from its folder name.
    pub shape: LevelShape,
    /// The region file headers kept for far readers (`far_record`).
    pub(super) far_regions: far_regions::FarRegions,
    /// Saved chunks decoded for far terrain, shared by its sampling threads
    /// (`lod_surfaces_shared`), oldest first.
    lod_decoded: Mutex<LodDecoded>,
    /// What older chunks are upgraded with, found on the first one read.
    datafix_context: OnceLock<Option<DatafixContext>>,
    /// The Overworld's old-world blending cache, read when a save is requested
    /// (installed where `SavedOldWorld` is made for this same folder).
    blending_lookup: OnceLock<BlendingLookup>,
}

impl SaveDir {
    /// This dimension's server-owned mod attachments: `data/sunrise/attachments.dat`, Sunrise's
    /// own namespace. A file that doesn't read is moved aside to `attachments.dat.corrupt`, with
    /// a warning, and the level opens without them (as vanilla's `SavedDataStorage` starts a
    /// saved data over when its file is broken), so the next save can't overwrite it.
    pub fn load_attachments(&self) -> sunrise_mod_runtime::attachments::Attachments {
        let mut path = self.attachments_path();
        let mut read = try_read_dat(&path);
        if matches!(read, Err(DatIssue::Missing)) {
            // A dimension Horizon saved: its file, written under Sunrise's name at the next save.
            path = horizon_names::horizon_attachments_path(&self.root);
            read = try_read_dat(&path);
        }
        let data = match read {
            Err(DatIssue::Missing) => return Default::default(),
            Ok(root) => match root.get("data") {
                Some(Tag::Compound(data)) => Ok(data.clone()),
                _ => Err("no data compound".to_owned()),
            },
            Err(DatIssue::Unreadable(why)) => Err(why),
        };
        data.map(sunrise_mod_runtime::attachments::Attachments::from_nbt)
            .unwrap_or_else(|why| {
                let aside = path.with_extension("dat.corrupt");
                log::warn!(
                    "{}: {why}; mod attachments start over, the file is kept as {}",
                    path.display(),
                    aside.display()
                );
                let _ = fs::rename(&path, &aside);
                Default::default()
            })
    }

    /// Save this dimension's server-owned mod attachments; nothing while it has none and never had.
    pub fn save_attachments(
        &self,
        attachments: &sunrise_mod_runtime::attachments::Attachments,
    ) -> Result<()> {
        let path = self.attachments_path();
        let data = match attachments.try_to_nbt() {
            Ok(data) => data,
            Err(error) => {
                log::error!(
                    "Sunrise: refusing level attachments save at {}: {error}",
                    path.display()
                );
                return Err(error.into());
            }
        };
        // Emptied after a Horizon file was read: an empty file under Sunrise's name keeps it unread.
        if data.is_empty()
            && !path.exists()
            && !horizon_names::horizon_attachments_path(&self.root).exists()
        {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::Compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }

    fn attachments_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("sunrise")
            .join("attachments.dat")
    }

    fn entity_metadata_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("sunrise")
            .join("entity_metadata.dat")
    }

    /// Versioned, engine-owned entity metadata (`data/sunrise/entity_metadata.dat`):
    /// returns the dragon UUIDs whose `living_death` was vetoed and whether the file
    /// exists on disk. Never reads arbitrary raw entity NBT fields.
    pub fn load_dragon_death_canceled(&self) -> (HashSet<[i32; 4]>, bool) {
        const VERSION: i32 = 1;
        let path = self.entity_metadata_path();
        match try_read_dat(&path) {
            Err(DatIssue::Missing) => (HashSet::new(), false),
            Ok(root) => {
                if root.get("version") != Some(&Tag::Int(VERSION)) {
                    log::warn!(
                        "{}: unsupported entity metadata version; ignoring persisted dragon vetoes",
                        path.display()
                    );
                    return (HashSet::new(), true);
                }
                let mut out = HashSet::new();
                if let Some(Tag::List(entries)) = root.get("dragon_death_canceled") {
                    for entry in entries {
                        if let Tag::IntArray(words) = entry
                            && words.len() == 4
                        {
                            out.insert([words[0], words[1], words[2], words[3]]);
                        }
                    }
                }
                (out, true)
            }
            Err(DatIssue::Unreadable(why)) => {
                let aside = path.with_extension("dat.corrupt");
                log::warn!(
                    "{}: {why}; engine entity metadata starts over, the file is kept as {}",
                    path.display(),
                    aside.display()
                );
                let _ = fs::rename(&path, &aside);
                (HashSet::new(), false)
            }
        }
    }

    /// Persist versioned, engine-owned dragon death-veto metadata; removes the file
    /// when empty and writes nothing when already absent.
    pub fn save_dragon_death_canceled(&self, uuids: &HashSet<[i32; 4]>) -> Result<()> {
        const VERSION: i32 = 1;
        let path = self.entity_metadata_path();
        if uuids.is_empty() {
            if let Err(err) = fs::remove_file(&path)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                return Err(err.into());
            }
            return Ok(());
        }
        let mut sorted: Vec<[i32; 4]> = uuids.iter().copied().collect();
        sorted.sort_unstable();
        let list = sorted
            .into_iter()
            .map(|u| Tag::IntArray(u.to_vec()))
            .collect();
        write_dat(
            &path,
            Tag::compound([
                ("DataVersion", Tag::Int(DATA_VERSION)),
                ("version", Tag::Int(VERSION)),
                ("dragon_death_canceled", Tag::List(list)),
            ]),
        )
    }

    /// The saves of a level of `shape` at `root` (its dimension folder).
    pub fn new(root: PathBuf, shape: LevelShape, items: Option<Arc<crate::items::Items>>) -> Self {
        Self {
            save_reporting: writes::SaveReporting::new(
                root.clone(),
                "minecraft:overworld",
                Arc::default(),
                crate::chunk_io::PROCESS_REPORTS.clone(),
            ),
            root,
            items,
            regions: Default::default(),
            entity_regions: Default::default(),
            failed_entity_reads: Default::default(),
            poi_regions: Default::default(),
            pending: Default::default(),
            sequence: AtomicU64::new(0),
            disk: OnceLock::new(),
            writer: Mutex::new(None),
            outstanding: Default::default(),
            tables: OnceLock::new(),
            professions: Default::default(),
            server_owned: false,
            shape,
            far_regions: Default::default(),
            lod_decoded: Default::default(),
            datafix_context: OnceLock::new(),
            blending_lookup: OnceLock::new(),
        }
    }
    /// Share the world's record diagnostic sink across its levels, before any writes.
    pub fn set_save_reporting(
        &mut self,
        world_root: PathBuf,
        dimension: &'static str,
        reporter: Arc<crate::chunk_io::SaveFailureReporter>,
        inbox: Arc<crate::chunk_io::Reports>,
    ) {
        self.save_reporting = writes::SaveReporting::new(world_root, dimension, reporter, inbox);
    }

    pub fn chunk_reports(&self) -> &Arc<crate::chunk_io::Reports> {
        &self.save_reporting.inbox
    }

    pub fn save_failure_reporter(&self) -> &Arc<crate::chunk_io::SaveFailureReporter> {
        &self.save_reporting.reporter
    }

    pub(super) fn region_storage(&self, folder: Folder) -> RegionStorage<'_> {
        RegionStorage {
            root: &self.root,
            folder,
            cache: match folder {
                Folder::Region => &self.regions,
                Folder::Entities => &self.entity_regions,
                Folder::Poi => &self.poi_regions,
            },
            reporting: &self.save_reporting,
        }
    }

    pub(super) fn items_for_save(&self) -> Result<&Arc<crate::items::Items>> {
        self.items
            .as_ref()
            .ok_or_else(|| "cannot save chunks without a loaded item catalog".into())
    }
    /// Mark this as a server-owned world save, not a remote cache.
    pub fn mark_server_owned(&mut self) {
        self.server_owned = true;
    }
    /// Lets saves of this folder take an old chunk's calculated blending data
    /// (`lookup` must not calculate). Set once, before the folder saves.
    pub fn set_blending_lookup(&self, lookup: BlendingLookup) {
        let _ = self.blending_lookup.set(lookup);
    }
    /// The calculated blending data of the old chunk at `x, z`, if this folder
    /// has a lookup and its cache holds one.
    pub fn calculated_blending(&self, x: i32, z: i32) -> Option<Arc<BlendingData>> {
        self.blending_lookup.get().and_then(|lookup| lookup(x, z))
    }
    /// 26.3's TicketStorage: persistent tickets live in each dimension's
    /// data/minecraft/chunk_tickets.dat, with ChunkPos.CODEC's [x, z] int array.
    pub fn load_forced_chunks(&self) -> HashSet<(i32, i32)> {
        let Some(data) = read_saved_data(&self.root, "chunk_tickets") else {
            return HashSet::new();
        };
        let Some(Tag::List(tickets)) = data.get("tickets") else {
            return HashSet::new();
        };
        tickets
            .iter()
            .filter(|t| is_forced_ticket(t))
            .filter_map(|t| match t.get("chunk_pos")? {
                Tag::IntArray(pos) if pos.len() == 2 => Some((pos[0], pos[1])),
                _ => None,
            })
            .collect()
    }
    /// The saved `PORTAL` tickets, for `TicketStorage.activateAllDeactivatedTickets`
    /// (`Ticket.CODEC`: a non-negative level, `ticks_left` 0 when missing).
    pub fn load_portal_tickets(&self) -> Vec<crate::ticket_levels::PortalTicket> {
        let Some(Tag::List(tickets)) =
            read_saved_data(&self.root, "chunk_tickets").and_then(|d| d.get("tickets").cloned())
        else {
            return Vec::new();
        };
        tickets
            .iter()
            .filter(|t| is_portal_ticket(t))
            .filter_map(|t| {
                let chunk = match t.get("chunk_pos")? {
                    Tag::IntArray(pos) if pos.len() == 2 => (pos[0], pos[1]),
                    _ => return None,
                };
                let level = match t.get("level")? {
                    &Tag::Int(level) if level >= 0 => level,
                    _ => return None,
                };
                let ticks_left = match t.get("ticks_left") {
                    None => 0,
                    Some(&Tag::Long(n)) => n,
                    Some(&Tag::Int(n)) => n.into(),
                    Some(&Tag::Short(n)) => n.into(),
                    Some(&Tag::Byte(n)) => n.into(),
                    Some(_) => return None,
                };
                Some(crate::ticket_levels::PortalTicket {
                    chunk,
                    level,
                    ticks_left,
                })
            })
            .collect()
    }
    /// `TicketStorage.packTickets`: the forced chunks and the `PORTAL` tickets
    /// the level holds now, in place of the ones saved before.
    pub fn save_chunk_tickets(
        &self,
        forced: &HashSet<(i32, i32)>,
        portal: &[crate::ticket_levels::PortalTicket],
    ) -> Result<()> {
        if forced.is_empty()
            && portal.is_empty()
            && !saved_data_path(&self.root, "chunk_tickets").exists()
        {
            return Ok(());
        }
        let mut data = compound_or_empty(read_saved_data(&self.root, "chunk_tickets"));
        // Preserve persistent tickets Sunrise does not simulate.
        let mut tickets = match data.get("tickets") {
            Some(Tag::List(tickets)) => tickets
                .iter()
                .filter(|t| !is_forced_ticket(t) && !is_portal_ticket(t))
                .cloned()
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        let mut chunks: Vec<_> = forced.iter().copied().collect();
        chunks.sort_unstable();
        tickets.extend(chunks.into_iter().map(|(x, z)| {
            Tag::compound([
                ("type", Tag::String("minecraft:forced".into())),
                ("level", Tag::Int(31)), // ChunkMap.FORCED_TICKET_LEVEL: ENTITY_TICKING.
                ("chunk_pos", Tag::IntArray(vec![x, z])),
            ])
        }));
        tickets.extend(portal.iter().map(|t| {
            let mut ticket = Tag::compound([
                ("type", Tag::String("minecraft:portal".into())),
                ("level", Tag::Int(t.level)),
                ("chunk_pos", Tag::IntArray(vec![t.chunk.0, t.chunk.1])),
            ]);
            // `optionalFieldOf("ticks_left", 0L)` leaves out its default.
            if t.ticks_left != 0 {
                put(&mut ticket, "ticks_left", Tag::Long(t.ticks_left));
            }
            ticket
        }));
        put(&mut data, "tickets", Tag::List(tickets));
        write_saved_data(&self.root, "chunk_tickets", data)
    }
    /// The chunk tickets of mods (`data/sunrise/mod_chunk_tickets.dat`), kept
    /// out of vanilla's `chunk_tickets.dat` because vanilla decodes its ticket
    /// types through a registry. Each is saved under its controller's id;
    /// `slot_of` gives the slot of an id a loaded mod registered. A ticket of
    /// any other id (a mod that is gone, or a file from before controllers) is
    /// dropped with a warning, and one that doesn't read is skipped.
    pub fn load_mod_tickets(&self, slot_of: impl Fn(&str) -> Option<u32>) -> Vec<ModTicket> {
        let Some(root) = read_dat(&self.mod_tickets_path()) else {
            return Vec::new();
        };
        let Some(Tag::List(tickets)) = root.get("data").and_then(|data| data.get("tickets")) else {
            return Vec::new();
        };
        let mut dropped = std::collections::BTreeSet::new();
        let loaded = tickets
            .iter()
            .filter_map(|tag| {
                let Some((controller, ticket)) = mod_ticket_from_nbt(tag) else {
                    log::warn!("Dropping a saved mod chunk ticket that has no readable controller");
                    return None;
                };
                let Some(slot) = slot_of(&controller) else {
                    dropped.insert(controller);
                    return None;
                };
                Some(ModTicket {
                    controller: slot,
                    ..ticket
                })
            })
            .collect();
        for controller in dropped {
            log::warn!(
                "Dropping the saved chunk tickets of ticket controller {controller}: no loaded mod registers it"
            );
        }
        loaded
    }
    /// Save the tickets of mods under their controllers' ids (`id_of` gives the id of a slot);
    /// nothing while there are none and never were.
    pub fn save_mod_tickets(
        &self,
        tickets: &ModTickets,
        id_of: impl Fn(u32) -> Option<&'static str>,
    ) -> Result<()> {
        let path = self.mod_tickets_path();
        let list: Vec<Tag> = tickets
            .to_save()
            .filter_map(|t| Some(mod_ticket_to_nbt(t, id_of(t.controller)?)))
            .collect();
        if list.is_empty() && !path.exists() {
            return Ok(());
        }
        write_dat(
            &path,
            Tag::compound([
                ("data", Tag::compound([("tickets", Tag::List(list))])),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        )
    }
    fn mod_tickets_path(&self) -> PathBuf {
        self.root
            .join("data")
            .join("sunrise")
            .join("mod_chunk_tickets.dat")
    }
    pub(super) fn region_path(&self, folder: &str, rx: i32, rz: i32) -> PathBuf {
        region_path(&self.root, folder, rx, rz)
    }
    /// The chunks of a region file that an older version generated, as
    /// `IOWorker.createOldDataForRegion` finds them (bit `x + z * 32`): saved
    /// before DataVersion 4997, or with `blending_data`.
    pub fn old_chunk_bits(&self, rx: i32, rz: i32) -> [u64; 16] {
        let mut bits = [0u64; 16];
        let Ok(region) = RegionFile::open(self.region_path("region", rx, rz)) else {
            return bits;
        };
        let present: Vec<ChunkPos> = region.chunks().map(|(pos, _)| pos).collect();
        drop(region);
        for pos in present {
            // `IOWorker` reads the record as saved, before any upgrade.
            let Some(root) = self.read_record_raw("region", pos.x, pos.z) else {
                continue;
            };
            if record_is_old(&root) {
                let index = ((pos.x & 31) + (pos.z & 31) * 32) as usize;
                bits[index / 64] |= 1 << (index % 64);
            }
        }
        bits
    }
    /// Whether the saved chunk at `x, z` is old ([`Self::old_chunk_bits`]'s test), from its
    /// record alone: false for a chunk not saved.
    pub fn is_old_chunk(&self, x: i32, z: i32) -> bool {
        self.read_record_raw("region", x, z)
            .is_some_and(|root| record_is_old(&root))
    }
    /// A saved chunk brought to the current version, for the readers of its
    /// fields that `load` does not decode (`blending_data`).
    pub fn raw_chunk(&self, x: i32, z: i32) -> Option<NamedTag> {
        self.read_record("region", x, z)
    }
    /// The references a saved chunk keeps for its structure start `name`
    /// (`StructureStart.getReferences`, saved as `references`): 0 for a chunk
    /// not saved yet, a start it does not have or one without the field.
    pub fn start_references(&self, x: i32, z: i32, name: &str) -> i32 {
        let Some(root) = self.read_record("region", x, z) else {
            return 0;
        };
        match root
            .tag
            .get("structures")
            .and_then(|s| s.get("starts"))
            .and_then(|s| s.get(name))
            .and_then(|s| s.get("references"))
        {
            Some(Tag::Int(n)) => *n,
            _ => 0,
        }
    }
    /// A record as saved: the pending write first, then the region file.
    pub(super) fn read_record_raw(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        self.try_read_record_raw(folder, x, z, Record::tag_for_metadata)
            .inspect_err(|e| {
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, e)
            })
            .ok()
            .flatten()
    }
    /// `read_record_raw`, telling a record that isn't there (`Ok(None)`) from a read that failed.
    fn try_read_record_raw(
        &self,
        folder: &str,
        x: i32,
        z: i32,
        read_pending: fn(&Record) -> Option<Arc<NamedTag>>,
    ) -> Result<Option<NamedTag>> {
        let kind = Folder::of(folder);
        let pending = self
            .pending
            .lock()
            .map_err(|_| "pending writes poisoned")?
            .get(&(kind, x, z))
            .map(|(_, r)| r.clone());
        if let Some(record) = pending
            && let Ok(tag) = compression_pool::catch_encoding_panic(|| Ok(read_pending(&record)))
        {
            return Ok(tag.map(|t| (*t).clone()));
        }
        read_region_record(self.region_storage(kind), x, z)
    }
    /// A record brought to the current version (`upgrade_record`); `Err` when that failed.
    fn try_read_record(
        &self,
        folder: &str,
        x: i32,
        z: i32,
        read_pending: fn(&Record) -> Option<Arc<NamedTag>>,
    ) -> Result<Option<NamedTag>> {
        let Some(mut record) = self.try_read_record_raw(folder, x, z, read_pending)? else {
            return Ok(None);
        };
        self.upgrade_record(Folder::of(folder), &mut record)?;
        Ok(Some(record))
    }
    /// A record brought to the current version (`upgrade_record`).
    pub(super) fn read_record(&self, folder: &str, x: i32, z: i32) -> Option<NamedTag> {
        self.try_read_record(folder, x, z, Record::tag_for_metadata)
            .inspect_err(|e| {
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, e)
            })
            .ok()
            .flatten()
    }
    pub(super) fn upgrade_record(&self, folder: Folder, record: &mut NamedTag) -> Result<()> {
        upgrade_record(folder, record, || {
            self.datafix_context
                .get_or_init(|| DatafixContext::of(&self.root))
                .as_ref()
        })
    }
    /// `ChunkMap.handleChunkLoadFailure`: once this chunk's read failure has been reported,
    /// let generation replace it, as `ChunkMap.createEmptyChunk` does.
    pub fn load(&self, x: i32, z: i32, registry: &BlockRegistry, biomes: &Biomes) -> Option<Chunk> {
        let mut chunk = self.try_load(x, z, registry, biomes).ok().flatten()?;
        self.load_disk_attachments(&mut chunk, mod_api::ChunkDiskKind::Full);
        Some(chunk)
    }
    /// `load`, telling a chunk that isn't saved as a full chunk (`Ok(None)`) from a read that
    /// failed (`Err`): a later read may succeed.
    pub(super) fn try_load(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> std::result::Result<Option<Chunk>, ()> {
        let Some(root) = self
            .try_read_record("region", x, z, Record::tag)
            .map_err(|e| {
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, &e)
            })?
        else {
            return Ok(None);
        };
        let tables = self.tables.get().map(|t| &**t);
        let Some(mut chunk) = decode_with(&root, registry, biomes, tables, self.shape.column())
        else {
            return Ok(None);
        };
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        chunk.entities = sunrise_world::chunk::legacy_entities(&root.tag);
        Ok(Some(chunk))
    }
    /// Read the last saved state of full chunk `(x, z)` from pending writes or its `.mca` file
    /// directly read-only, without touching gameplay `RegionCache`, loading entities, or firing
    /// mod hooks. Safe to call concurrently from background worker threads.
    pub fn read_saved_chunk_readonly(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<Option<Chunk>> {
        let pending = self
            .pending
            .lock()
            .map_err(|_| "pending writes poisoned")?
            .get(&(Folder::Region, x, z))
            .map(|(_, r)| r.clone());
        let mut root = if let Some(record) = pending {
            let tag = compression_pool::catch_encoding_panic(|| Ok(record.tag_for_metadata()))?;
            let Some(tag) = tag else {
                return Ok(None);
            };
            (*tag).clone()
        } else {
            let Some(root) =
                region::read_region_record_readonly(self.region_storage(Folder::Region), x, z)?
            else {
                return Ok(None);
            };
            root
        };
        self.upgrade_record(Folder::Region, &mut root)?;
        let Some(mut chunk) = decode_readonly_full(&root, (x, z), registry, biomes, self.shape)?
        else {
            return Ok(None);
        };
        load_light(&root.tag, &mut chunk, self.shape.sky_light);
        Ok(Some(chunk))
    }
    /// List chunk coordinates present in region `(rx, rz)`'s 8 KiB sector header unioned with
    /// pending unflushed writes (excluding pending deletes), without decompressing chunk payloads.
    pub fn saved_region_chunks(&self, rx: i32, rz: i32) -> Result<Vec<[i32; 2]>> {
        // Snapshot pending writes BEFORE reading the region header so a chunk flushed between the
        // two steps cannot be missed (`writes.rs` removes from `pending` only after writing disk).
        let mut pending_added = Vec::new();
        let mut pending_deleted = Vec::new();
        {
            let pending = self.pending.lock().map_err(|_| "pending writes poisoned")?;
            for (&(folder, x, z), (_, record)) in pending.iter() {
                if folder == Folder::Region && (x >> 5) == rx && (z >> 5) == rz {
                    if matches!(record.source(), Record::Delete) {
                        pending_deleted.push([x, z]);
                    } else {
                        pending_added.push([x, z]);
                    }
                }
            }
        }
        let mut chunks =
            region::list_region_chunks_readonly(self.region_storage(Folder::Region), rx, rz)?;
        if !pending_deleted.is_empty() {
            chunks.retain(|pos| !pending_deleted.contains(pos));
        }
        chunks.extend(pending_added);
        chunks.sort_unstable();
        chunks.dedup();
        Ok(chunks)
    }
    /// A proto chunk saved before it became final (`decode_proto`).
    pub fn load_proto(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<SavedProto> {
        let mut proto = self.decode_saved_proto(x, z, registry, biomes)?;
        self.load_disk_attachments(&mut proto.column, mod_api::ChunkDiskKind::Proto);
        Some(proto)
    }

    pub(super) fn decode_saved_proto(
        &self,
        x: i32,
        z: i32,
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Option<SavedProto> {
        let root = self.read_record("region", x, z)?;
        let tables = self.tables.get().map(|t| &**t);
        decode_proto(&root, registry, biomes, tables, self.shape.column())
    }
    /// Queue proto chunks for the writer thread; loads see them meanwhile.
    pub fn save_protos(
        &self,
        protos: Vec<SavedProto>,
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<()> {
        biomes.ensure_saveable()?;
        let encoding = Arc::new(Encoding {
            items: self.items_for_save()?.clone(),
            registry,
            biomes: biomes.clone(),
            tables: self.tables.get().cloned(),
            shape: self.shape,
        });
        let records = protos
            .into_iter()
            .map(|p| {
                (
                    (p.column.x, p.column.z),
                    Record::Proto(Arc::new(p), encoding.clone()),
                )
            })
            .collect();
        self.queue(Folder::Region, records, false)
    }
    /// `EntityStorage.loadEntities`: read `entities/` separately from the terrain.
    /// An absent record is empty; a failed read leaves the entity chunk pending until the level closes.
    pub fn load_entities(&self, x: i32, z: i32) -> Result<Vec<Tag>> {
        if self.failed_entity_reads.lock().unwrap().contains(&(x, z)) {
            return Err("an earlier entity chunk read failed".into());
        }
        let record = self
            .try_read_record("entities", x, z, Record::tag_for_metadata)
            .inspect_err(|e| {
                self.failed_entity_reads.lock().unwrap().insert((x, z));
                log::error!("Failed to load entity chunk {x},{z}: {e}");
                self.chunk_reports()
                    .report_load_failure(&self.root, x, z, e);
            })?;
        let mut out = Vec::new();
        if let Some(root) = record
            && let Some(Tag::List(list)) = root.tag.get("Entities")
        {
            out.extend(list.iter().cloned());
        }
        Ok(out)
    }
    /// Write `chunks` into their region files and wait until they are on disk.
    pub fn save(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &BlockRegistry,
        biomes: &Biomes,
    ) -> Result<usize> {
        biomes.ensure_saveable()?;
        let items = self.items_for_save()?;
        if let Some(result) = self.save_disk_sync(chunks, registry, biomes, items) {
            return result;
        }
        let tables = self.tables.get().map(|t| &**t);
        let records = chunks
            .iter()
            .filter_map(|c| {
                let key = (c.x, c.z);
                match compression_pool::catch_encoding_panic(|| {
                    let blending = self.calculated_blending(c.x, c.z);
                    encode_saved(
                        c,
                        registry,
                        biomes,
                        tables,
                        self.shape,
                        blending.as_deref(),
                        items,
                    )
                    .map_err(std::io::Error::other)
                }) {
                    Ok(tag) => Some((key, Some(tag))),
                    Err(error) => {
                        self.report_record_failure(Folder::Region, key, &error);
                        None
                    }
                }
            })
            .collect();
        self.write_records("region", records, true)?;
        self.save_poi(chunks, true)?;
        Ok(chunks.len())
    }
    pub(super) fn save_poi(&self, chunks: &[Arc<Chunk>], wait: bool) -> Result<()> {
        if self.tables.get().is_none() {
            return Ok(());
        }
        self.queue(Folder::Poi, self.poi_records(chunks), wait)
    }

    fn poi_records(&self, chunks: &[Arc<Chunk>]) -> Vec<((i32, i32), Record)> {
        let Some(tables) = self.tables.get() else {
            return Vec::new();
        };
        let professions = self
            .professions
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        chunks
            .iter()
            .map(|c| {
                (
                    (c.x, c.z),
                    Record::Poi(c.clone(), tables.clone(), professions.clone()),
                )
            })
            .collect()
    }

    /// `save` on the writer thread: returns before the files are written;
    /// loads see the chunks meanwhile.
    pub fn save_later(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
    ) -> Result<usize> {
        self.enqueue_chunk_saves(chunks, registry, biomes, None)?;
        Ok(chunks.len())
    }

    fn enqueue_chunk_saves(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
        eviction: Option<&EvictionWrites>,
    ) -> Result<()> {
        let encoding = self.encoding_for_save_later(chunks, registry, biomes)?;
        let records = chunks
            .iter()
            .map(|c| {
                let blending = self.calculated_blending(c.x, c.z);
                (
                    (c.x, c.z),
                    Record::Chunk(c.clone(), encoding.clone(), blending),
                )
            })
            .collect();
        self.enqueue_records(Folder::Region, records, eviction)?;
        self.enqueue_records(Folder::Poi, self.poi_records(chunks), eviction)
    }

    /// Save the records required by an idle column eviction and wait for queued
    /// writes to finish without forcing every cached region (`sync_all`).
    /// Returns the columns whose own records were refused or skipped; if the
    /// writer failed, eviction cannot assume any column was written.
    pub fn save_eviction(
        &self,
        chunks: &[Arc<Chunk>],
        registry: &'static BlockRegistry,
        biomes: &Arc<Biomes>,
        entities: Vec<((i32, i32), Vec<Tag>)>,
    ) -> Result<HashSet<(i32, i32)>> {
        let eviction = EvictionWrites::default();
        if !chunks.is_empty() {
            self.enqueue_chunk_saves(chunks, registry, biomes, Some(&eviction))?;
        }
        let records = self.entity_records(entities, Some(&eviction));
        self.enqueue_records(Folder::Entities, records, Some(&eviction))?;
        self.wait_for_writes()?;
        Ok(eviction.refused())
    }

    /// Write the entity chunks (`EntityStorage`): an empty list deletes it.
    pub fn save_entities(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.queue(Folder::Entities, self.entity_records(chunks, None), true)
    }
    /// `save_entities` on the writer thread.
    pub fn save_entities_later(&self, chunks: Vec<((i32, i32), Vec<Tag>)>) -> Result<()> {
        self.queue(Folder::Entities, self.entity_records(chunks, None), false)
    }
    /// `PersistentEntitySectionManager.storeChunkSections`: never store a pending entity chunk.
    fn entity_records(
        &self,
        mut chunks: Vec<((i32, i32), Vec<Tag>)>,
        eviction: Option<&EvictionWrites>,
    ) -> Vec<((i32, i32), Record)> {
        {
            let failed = self.failed_entity_reads.lock().unwrap();
            if !failed.is_empty() {
                chunks.retain(|(key, _)| {
                    if failed.contains(key) {
                        if let Some(eviction) = eviction {
                            eviction.refuse(*key);
                        }
                        false
                    } else {
                        true
                    }
                });
            }
        }
        chunks
            .into_iter()
            .map(|((x, z), entities)| {
                let record = if entities.is_empty() {
                    Record::Delete
                } else {
                    Record::Tag(Arc::new(NamedTag {
                        name: "".into(),
                        tag: Tag::compound([
                            ("DataVersion", Tag::Int(DATA_VERSION)),
                            ("Position", Tag::IntArray(vec![x, z])),
                            ("Entities", Tag::List(entities)),
                        ]),
                    }))
                };
                ((x, z), record)
            })
            .collect()
    }
}

impl SaveDir {
    /// `lod_surfaces`, decoded once for all far sampling threads: each saved
    /// chunk lies under far tiles of several levels, sampled on any thread.
    pub(super) fn lod_surfaces_shared(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        if let Some((at, columns)) = self.lod_decoded.lock().unwrap().chunks.get(&(x, z))
            && at.elapsed() < LOD_DECODED_TTL
        {
            return Some(columns.clone());
        }
        let columns = self.lod_surfaces(x, z, assets, biomes)?;
        let mut decoded = self.lod_decoded.lock().unwrap();
        let fresh = (std::time::Instant::now(), columns.clone());
        if decoded.chunks.insert((x, z), fresh).is_none() {
            decoded.order.push_back((x, z));
            while decoded.order.len() > LOD_DECODED {
                if let Some(old) = decoded.order.pop_front() {
                    decoded.chunks.remove(&old);
                }
            }
        }
        Some(columns)
    }
    pub(super) fn lod_surfaces(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        let root = self.far_record(x, z)?;
        // Proto saves may still be missing decoration from their neighbors.
        if text(root.tag.get("Status"))
            .is_some_and(|status| status != "minecraft:full" && status != "full")
        {
            return None;
        }
        decode_lod_surfaces(
            &root,
            assets,
            biomes,
            self.shape.column(),
            self.shape.sky_light,
        )
    }
}
