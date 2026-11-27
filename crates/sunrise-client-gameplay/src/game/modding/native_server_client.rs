use super::Dim;
use mod_api::__private::game::*;
use mod_api::__private::{ClientCommandExecutor, RetainedCallback, RetainedFailure};
use mod_api::LevelRef;
use mod_api::PlayerRef;
use std::{collections::HashMap, sync::Arc};
use sunrise_command::{Arg, ArgType, Context, Dispatcher};
use sunrise_core::position::BlockPos;
use sunrise_game_data::text::{RED, Text};
use sunrise_gameplay_common::game::api_values::TryIntoApi;
use sunrise_gameplay_common::game::commands_common::{
    CommandsCommonGameExt1 as _, format_syntax_failure_context,
};
use sunrise_gameplay_common::game::machine_containers::Containers;
use sunrise_gameplay_common::game::native_server::{
    ClientCommandOutcome, PlayerRead,
    argument_kinds::{custom_value, nbt_value, parsed_type, positional_value},
};
use sunrise_items::items::ItemStack;
use sunrise_level::world::World;
use sunrise_sim::lan::abilities::PlayerAbilities;

pub(in crate::game) struct RegisteredClientExecutor {
    pub(in crate::game) callback: RetainedCallback,
    call: Option<Arc<ClientCommandExecutor>>,
}

impl RegisteredClientExecutor {
    pub(in crate::game) fn new(
        owner: &'static str,
        index: usize,
        usage: &str,
        call: Arc<ClientCommandExecutor>,
    ) -> Self {
        Self {
            callback: RetainedCallback::new(owner, index, "client_command.execute", usage),
            call: Some(call),
        }
    }

    pub(in crate::game) fn execute(
        &self,
        host: &mut dyn mod_api::ClientHost,
        invocation: &mod_api::CommandInvocation,
    ) -> Result<Option<i32>, RetainedFailure> {
        let call = self.call.as_ref().expect("live client executor");
        self.callback.call(|| call(host, invocation))
    }
}

impl Drop for RegisteredClientExecutor {
    fn drop(&mut self) {
        self.callback
            .dispose("client_command.drop", self.call.take());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::game) enum PendingClientCommandKind {
    Signed,
    Unattended,
}

/// An invocation path, shared by all submissions from the same executor. Use the retained
/// registration identity rather than a parsed node, which can change on a tree refresh.
pub(in crate::game) struct ClientCommandChain {
    pub(in crate::game) executor: RetainedCallback,
    pub(in crate::game) parent: Option<Arc<Self>>,
}

pub(in crate::game) struct PendingClientCommand {
    pub(in crate::game) command: String,
    pub(in crate::game) kind: PendingClientCommandKind,
    pub(in crate::game) chain: Arc<ClientCommandChain>,
}

/// The client's own native mod commands, bound to the current client dispatcher generation.
pub struct NativeClientCommandState {
    pub(in crate::game) dispatcher: Dispatcher,
    pub(in crate::game) executors: Vec<Option<Arc<RegisteredClientExecutor>>>,
    /// Present while an executor runs; nested slash submissions inherit this path and wait
    /// for the client event loop instead of re-entering synchronously.
    pub(in crate::game) active_chain: Option<Arc<ClientCommandChain>>,
    pub(in crate::game) pending_commands: Vec<PendingClientCommand>,
    /// Present while `client_chat` listeners run; a chat message a listener submits waits for
    /// the next event-loop wave instead of re-entering the hook synchronously.
    pub(in crate::game) chat_dispatching: bool,
    pub pending_chats: Vec<String>,
}

impl Default for NativeClientCommandState {
    fn default() -> Self {
        Self {
            dispatcher: Dispatcher::new(),
            executors: Vec::new(),
            active_chain: None,
            pending_commands: Vec::new(),
            chat_dispatching: false,
            pending_chats: Vec::new(),
        }
    }
}

impl NativeClientCommandState {
    pub(in crate::game) fn defer(&mut self, command: &str, kind: PendingClientCommandKind) -> bool {
        let Some(chain) = self.active_chain.clone() else {
            return false;
        };
        self.pending_commands.push(PendingClientCommand {
            command: command.to_owned(),
            kind,
            chain,
        });
        true
    }

    pub(in crate::game) fn install(
        &mut self,
        dispatcher: Dispatcher,
        executors: Vec<Option<RegisteredClientExecutor>>,
    ) {
        self.dispatcher = dispatcher;
        // A panicking retired callable may disable its replacement in the shared quarantine.
        self.executors = executors
            .into_iter()
            .map(|entry| entry.map(Arc::new))
            .collect();
    }
}

/// Helpers for server mods, kept as stable as the hook points.
// Native mods use these; nothing in Sunrise does.
#[allow(dead_code)]
impl crate::game::ClientGame<'_> {
    pub(in crate::game) fn player_discovered_recipes(
        &self,
        _player: PlayerRef,
    ) -> Option<Vec<String>> {
        None
    }

    /// Reads live `Remote::abilities` with predicted `flying` and `flying_speed` from the body.
    pub(in crate::game) fn client_abilities(&self) -> Option<PlayerAbilities> {
        let mut a = *self.remote_abilities()?;
        if let Some(body) = self.common.player.as_ref() {
            a.flying = body.flying;
            a.flying_speed = body.flying_speed;
        }
        Some(a)
    }

    pub(in crate::game) fn client_player(&self, uuid: u128) -> Option<PlayerRef> {
        let epoch = self.common.native_lifecycle.epoch()?;
        let remote = self
            .client
            .remote
            .as_ref()
            .filter(|r| r.logged_in && r.profile == uuid)?;
        Some(mod_api::__private::game::player_ref(
            mod_api::EntityRef::new(epoch, u32::try_from(remote.entity_id).ok()?),
            uuid,
        ))
    }

    pub(in crate::game) fn client_player_read_both(
        &self,
        player: PlayerRef,
    ) -> Option<PlayerRead<'_>> {
        (self.local_player()? == player).then(|| PlayerRead::Acting {
            mode: self
                .player_info_game_mode()
                .map(sunrise_command::args::GameMode::id),
            body: self.common.player.as_ref(),
            abilities: self.client_abilities(),
            last_death: self.common.level.last_death.as_ref(),
        })
    }

    pub(in crate::game) fn client_player_dimension_key(
        &self,
        player: PlayerRef,
    ) -> Option<mod_api::DimensionKey> {
        let epoch = self.common.native_lifecycle.epoch()?;
        if epoch != player.fields().entity.epoch() {
            return None;
        }
        (self.local_player()? == player).then(|| self.native_level_key())
    }

    pub(in crate::game) fn client_player_collision_flags(
        &self,
        player: PlayerRef,
    ) -> Result<Option<mod_api::CollisionFlags>, mod_api::LevelReadError> {
        Ok(self.client_player_read_both(player).and_then(|read| {
            let PlayerRead::Acting {
                body: Some(body), ..
            } = read
            else {
                return None;
            };
            let flags = body.collision;
            Some(mod_api::CollisionFlags::new(
                flags.horizontal,
                flags.minor,
                flags.vertical,
                flags.below,
            ))
        }))
    }

    pub(in crate::game) fn client_player_client_brand(&self, _player: PlayerRef) -> Option<&str> {
        None
    }

    /// `LocalPlayer.permissionLevel`, from the server's entity events 24 to 28.
    pub(in crate::game) fn player_permission_level(&self, player: PlayerRef) -> Option<u8> {
        (self.local_player()? == player).then_some(self.common.ui.registries.permission)
    }

    pub(in crate::game) fn client_tick_uses_server_sections(&self) -> bool {
        false
    }

    pub(in crate::game) fn client_crafting_remainder_hook(
        &self,
        _stack: &ItemStack,
        _remainder: &mut Option<ItemStack>,
    ) {
    }

    pub(in crate::game) fn client_world_table_items(
        &self,
    ) -> Option<&Arc<sunrise_items::items::Items>> {
        None
    }

    pub(in crate::game) fn client_lid_openers(&self) -> Option<&HashMap<(i32, i32, i32), i32>> {
        None
    }

    pub(in crate::game) fn client_signal_capability_site(
        &self,
        _at: mod_api::BlockSide,
    ) -> Option<mod_api::BlockCapabilitySite> {
        None
    }

    #[inline(always)]
    pub(in crate::game) fn client_level_containers<'w>(
        &'w self,
        world: &'w World,
    ) -> Containers<'w> {
        Containers {
            world,
            behaviours: self.common.res.behaviours(),
            items: self.items(),
            registry: self.common.pack.assets.registry,
            brewing: None,
            mobs: &self.common.mobs,
        }
    }

    pub(in crate::game) fn client_signal_container_size(
        &self,
        world: &World,
        at: mod_api::BlockSide,
    ) -> Option<usize> {
        (at.level.dimension() == self.native_level_key()).then_some(())?;
        self.client_level_containers(world).capability_size(at)
    }

    pub(in crate::game) fn client_signal_lend_container_stack(
        &self,
        world: &World,
        at: mod_api::BlockSide,
        slot: usize,
        read: &mut dyn for<'a> FnMut(mod_api::ItemStackView<'a>),
    ) {
        if at.level.dimension() != self.native_level_key() {
            return;
        }
        self.client_level_containers(world)
            .lend_capability_stack(at, slot, read);
    }

    pub(in crate::game) fn client_level_block_face_sturdy(
        &self,
        level: LevelRef,
        pos: [i32; 3],
        face: mod_api::BlockFace,
        support: mod_api::SupportType,
    ) -> Option<bool> {
        sunrise_gameplay_common::game::native_world_read_common::block_face_sturdy(
            self.client_level_view_world(level)?,
            self.common.res.behaviours(),
            self.common.res.collision(),
            pos,
            face,
            support,
        )
    }

    pub(in crate::game) fn client_signal_block_entity(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<mod_api::BlockEntityRef> {
        let world = self.client_level_view_world(level)?;
        sunrise_gameplay_common::game::native_block_entity_read::world_block_entity(
            world, level, pos,
        )
    }

    pub(in crate::game) fn client_signal_block_entity_data(
        &self,
        entity: mod_api::BlockEntityRef,
    ) -> Option<mod_api::Compound> {
        let world = self.client_level_view_world(entity.fields().level)?;
        sunrise_gameplay_common::game::native_block_entity_read::world_block_entity_data(
            world,
            self.common.pack.assets.registry,
            self.items(),
            entity,
        )
    }

    pub(in crate::game) fn client_level_is_raining_at(
        &self,
        level: LevelRef,
        pos: [i32; 3],
    ) -> Option<bool> {
        let world = self.client_level_view_world(level)?;
        if !world.is_loaded_at(pos[0], pos[1], pos[2]) {
            return None;
        }
        let (rain, _) = sunrise_gameplay_common::game::mod_api_host_common::weather_for_level(
            world.shape,
            level.dimension() == Dim::End.key(),
            world.weather.levels(1.0),
        );
        if rain <= 0.2 {
            return Some(false);
        }
        Some(sunrise_gameplay_common::game::native_rain_read::rain_at(
            world,
            self.common.res.facts(),
            self.common
                .world_biomes
                .as_deref()
                .unwrap_or_else(|| self.common.res.biomes()),
            self.common.res.behaviours(),
            Dim::from_id(level.dimension_id()),
            BlockPos::new(pos[0], pos[1], pos[2]),
        ))
    }

    fn client_command_item_stack(
        &self,
        sunrise_command::args::ItemInput { id, components, .. }: &sunrise_command::args::ItemInput,
        count: u32,
    ) -> Result<ItemStack, sunrise_command::CommandError> {
        let (damage, extra) = match components
            .as_deref()
            .map(sunrise_items::item_data::command_components)
        {
            Some(Ok(tag)) => sunrise_items::item_data::decode_components(Some(&tag)),
            Some(Err(e)) => return Err(sunrise_command::CommandError::new(Text::literal(e))),
            None => (0, None),
        };
        let stack = ItemStack {
            item: id.as_str().into(),
            count,
            damage,
            extra,
        };
        let max_stack = self.stack_max_size(&stack);
        if count > max_stack {
            return Err(sunrise_gameplay_common::game::commands_common::err(
                "arguments.item.overstacked",
                &[id, &max_stack],
            ));
        }
        Ok(stack)
    }

    /// Resolve argument kinds shared by server and client native commands.
    fn resolve_client_safe_argument(
        &self,
        ty: Option<&ArgType>,
        value: Arg,
        coords_position: impl FnOnce(&sunrise_command::args::Coordinates) -> [f64; 3],
        rotation: [f32; 2],
    ) -> Result<mod_api::CommandArgumentValue, sunrise_command::CommandError> {
        use mod_api::CommandArgumentValue as Value;
        use sunrise_gameplay_common::game::commands_common::err;
        Ok(match value {
            Arg::Str(text) if matches!(ty, Some(ArgType::Nbt)) => nbt_value(&text)?,
            Arg::Str(value) => Value::String(value),
            Arg::Int(value) => Value::Integer(value),
            Arg::Long(value) => Value::Long(value),
            Arg::Double(value) => Value::Number(value),
            Arg::Bool(value) => Value::Boolean(value),
            Arg::Coords(coordinates) => {
                positional_value(ty, &coordinates, coords_position(&coordinates), rotation)
            }
            Arg::Item(input) => Value::Item(
                self.client_command_item_stack(&input, 1)?
                    .try_into_api()
                    .expect("command item validation resolves registered items"),
            ),
            // `ResourceOrTagKeyArgument.Result`: `#` names a tag, else a resource.
            Arg::Resource(id) if matches!(ty, Some(ArgType::ResOrTagKey(_))) => {
                let (tag, name) = id
                    .strip_prefix('#')
                    .map_or((false, &*id), |name| (true, name));
                Value::ResourceOrTag {
                    tag,
                    id: mod_api::Identifier::parse(name).map_err(|_| err("command.failed", &[]))?,
                }
            }
            Arg::Resource(id) => {
                Value::Id(mod_api::Identifier::parse(&id).map_err(|_| err("command.failed", &[]))?)
            }
            Arg::Custom { kind, value } => custom_value(&kind, value),
            _ => unreachable!("the native tree contains only registered native argument kinds"),
        })
    }

    fn client_command_position(&self) -> [f64; 3] {
        match &self.common.player {
            Some(p) => [p.position.x, p.position.y, p.position.z],
            None => self.common.spawn.map_or([0.0, 64.0, 0.0], |s| {
                [s.x.floor(), s.y.floor(), s.z.floor()]
            }),
        }
    }

    fn resolve_client_native_command(
        &self,
        context: Context,
    ) -> Result<mod_api::CommandInvocation, sunrise_command::CommandError> {
        use mod_api::CommandArgument;
        let position = self.client_command_position();
        let rotation = [self.common.yaw, self.common.pitch];
        let mut arguments = Vec::with_capacity(context.args.len());
        for argument in context.args {
            let ty = parsed_type(
                &self.client.native_client_commands.dispatcher,
                &context.nodes,
                argument.range,
            );
            let value = self.resolve_client_safe_argument(
                ty,
                argument.value,
                |coordinates| coordinates.position(position, rotation),
                rotation,
            )?;
            arguments.push(CommandArgument::new(argument.name, value));
        }
        Ok(mod_api::CommandInvocation::new(
            context.literals,
            arguments,
            self.native_level_key(),
            position,
            rotation,
        ))
    }

    fn client_command_failure(&mut self, message: Text) {
        self.chat_line(Text::empty().append(message).with_color(RED));
    }

    fn client_command_syntax_failure(
        &mut self,
        error: &sunrise_command::CommandError,
        command: &str,
    ) {
        self.client_command_failure(error.message.clone());
        if let Some(context) = format_syntax_failure_context(error, command) {
            self.client_command_failure(context);
        }
    }

    /// `ClientCommandHandler.runCommand`: only Brigadier's unknown command and unknown argument
    /// outcomes leave the line to the server. All other failures stay on this client.
    pub(in crate::game) fn run_client_native_command(
        &mut self,
        line: &str,
    ) -> Option<ClientCommandOutcome> {
        self.run_client_native_command_in_chain(line, None)
    }

    pub(in crate::game) fn run_client_native_command_in_chain(
        &mut self,
        line: &str,
        ancestors: Option<Arc<ClientCommandChain>>,
    ) -> Option<ClientCommandOutcome> {
        if self.common.is_logical_server() {
            return None;
        }
        let parse = self
            .client
            .native_client_commands
            .dispatcher
            .parse(line, &self.common.ui.registries);
        let context = match parse.executable() {
            Ok((context, _)) => context.clone(),
            Err(error) => {
                if matches!(
                    error.key.as_str(),
                    "command.unknown.command" | "command.unknown.argument"
                ) {
                    return None;
                }
                self.client_command_syntax_failure(&error, line);
                return Some(ClientCommandOutcome::Failed);
            }
        };
        let Some(executor) = context
            .executor
            .filter(|&key| {
                self.client
                    .native_client_commands
                    .dispatcher
                    .owns_executor(key)
            })
            .and_then(|key| self.client.native_client_commands.executors.get(key.slot()))
            .and_then(Option::as_ref)
            .cloned()
        else {
            log::error!("client command has no executor in its dispatcher generation: {line}");
            self.client_command_failure(Text::translate("command.failed", &[]));
            return Some(ClientCommandOutcome::Failed);
        };
        if std::iter::successors(ancestors.as_deref(), |link| link.parent.as_deref())
            .any(|link| link.executor == executor.callback)
        {
            log::warn!("recursive client command refused: {line}");
            self.client_command_failure(Text::translate("command.failed", &[]));
            return Some(ClientCommandOutcome::Failed);
        }
        let invocation = match self.resolve_client_native_command(context) {
            Ok(invocation) => invocation,
            Err(error) => {
                self.client_command_syntax_failure(&error, line);
                return Some(ClientCommandOutcome::Failed);
            }
        };
        self.client.native_client_commands.active_chain = Some(Arc::new(ClientCommandChain {
            executor: executor.callback.clone(),
            parent: ancestors,
        }));
        let executed = executor.execute(self, &invocation);
        self.client.native_client_commands.active_chain = None;
        match executed {
            Ok(Some(result)) => Some(ClientCommandOutcome::Success(result)),
            failure => {
                match failure {
                    Err(why) => log::warn!("client command failed: {why:?}"),
                    _ => log::warn!(
                        "client command failed: mod {} returned no result",
                        executor.callback.owner()
                    ),
                }
                self.client_command_failure(Text::translate("command.failed", &[]));
                Some(ClientCommandOutcome::Failed)
            }
        }
    }

    /// Dispatch one event-loop wave. The next submissions retain this wave's executor
    /// ancestry but cannot run until the event loop returns again.
    pub fn drain_pending_client_commands(&mut self) {
        if self.client.native_client_commands.active_chain.is_some()
            || self.client.native_client_commands.chat_dispatching
        {
            return;
        }
        for message in std::mem::take(&mut self.client.native_client_commands.pending_chats) {
            if self.client.remote.is_none() {
                break;
            }
            let _ = self.send_chat(&message);
        }
        let pending = std::mem::take(&mut self.client.native_client_commands.pending_commands);
        for item in pending {
            if self.client.remote.is_none() {
                break;
            }
            let _ = self.submit_client_command(&item.command, item.kind, Some(item.chain));
        }
    }
}

use sunrise_gameplay_common::game::tables::GameItems as _;

use sunrise_gameplay_common::game::game_mode_common::GameModeCommonGameExt1 as _;

use sunrise_gameplay_common::game::native_entity_refs_common::NativeEntityRefsCommonGameExt1 as _;

use sunrise_gameplay_common::game::player_identity::PlayerIdentityGameExt1 as _;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct MachineArgument;

    impl mod_api::CommandArgumentType for MachineArgument {
        fn kind(&self) -> mod_api::CommandArgumentKind {
            mod_api::CommandArgumentKind::Word
        }

        fn parse(&self, input: &str) -> Result<String, mod_api::CommandArgumentError> {
            match input {
                "press" | "furnace" => Ok(input.to_owned()),
                _ => Err(mod_api::CommandArgumentError::new(
                    mod_api::Text::translate(
                        "test.commands.machine.unknown",
                        vec![mod_api::Text::literal(input)],
                    ),
                )),
            }
        }

        fn suggest(&self, request: &mod_api::SuggestionRequest) -> Vec<mod_api::Suggestion> {
            ["press", "furnace"]
                .into_iter()
                .filter(|machine| machine.starts_with(request.prefix.as_str()))
                .map(|machine| request.suggest(machine))
                .collect()
        }
    }

    #[test]
    #[ignore = "daily"]
    fn client_commands_execute_bound_typed_invocations_contain_panics_and_reject_stale_generations()
    {
        use mod_api::prelude::NetworkClientHost;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

        struct DropBomb(Arc<AtomicBool>);
        impl Drop for DropBomb {
            fn drop(&mut self) {
                if !self.0.swap(true, Ordering::Relaxed) {
                    panic!("client command capture destructor fixture");
                }
            }
        }

        struct ClientCommandsMod {
            heard: Arc<Mutex<Vec<mod_api::CommandInvocation>>>,
            sibling: Arc<Mutex<Vec<mod_api::CommandInvocation>>>,
            recursed: Arc<AtomicUsize>,
            ping_pong: Arc<Mutex<Vec<&'static str>>>,
            dropped: Arc<AtomicBool>,
        }

        #[mod_api::__subscriptions]
        impl mod_api::Hooks for ClientCommandsMod {
            fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
                r.register_command_argument("machine", MachineArgument);
            }

            fn register_client_commands(
                &self,
                _: &mut dyn mod_api::ClientHost,
                event: &mut mod_api::RegisterClientCommandsEvent<'_>,
            ) {
                let heard = self.heard.clone();
                event.commands.push(
                    mod_api::Command::new(
                        "cmach <machine:client_cmd_fixture:machine> <pos:block_pos> [count:integer]",
                    )
                    .executes_client(move |_, invocation| {
                        heard.lock().unwrap().push(invocation.clone());
                        Some(1)
                    }),
                );
                event.commands.push(
                    mod_api::Command::new("cpanic")
                        .executes_client(|_, _| panic!("client command panic fixture")),
                );
                let sibling = self.sibling.clone();
                event.commands.push(
                    mod_api::Command::new("csibling <word:word>").executes_client(
                        move |_, invocation| {
                            sibling.lock().unwrap().push(invocation.clone());
                            Some(1)
                        },
                    ),
                );
                let recursed = self.recursed.clone();
                event
                    .commands
                    .push(
                        mod_api::Command::new("crecurse").executes_client(move |host, _| {
                            recursed.fetch_add(1, Ordering::Relaxed);
                            assert!(host.send_chat("/crecurse").is_ok());
                            Some(1)
                        }),
                    );
                let ping = self.ping_pong.clone();
                let pong = self.ping_pong.clone();
                event.commands.extend([
                    mod_api::Command::new("cping").executes_client(move |host, _| {
                        ping.lock().unwrap().push("ping");
                        for _ in 0..2 {
                            assert!(host.send_chat("/cpong").is_ok());
                        }
                        Some(1)
                    }),
                    mod_api::Command::new("cpong").executes_client(move |host, _| {
                        pong.lock().unwrap().push("pong");
                        for _ in 0..2 {
                            assert!(host.send_chat("/cping").is_ok());
                        }
                        assert!(host.send_chat("/csibling nested").is_ok());
                        Some(1)
                    }),
                ]);
                let bomb = DropBomb(self.dropped.clone());
                event.commands.extend([
                    mod_api::Command::new("capture").executes_client(move |_, _| {
                        let _capture = &bomb;
                        Some(1)
                    }),
                    mod_api::Command::new("shared local <value:integer>")
                        .executes_client(|_, _| Some(42)),
                    mod_api::Command::new("shared refused").executes_client(|_, _| None),
                ]);
                event
                    .commands
                    .push(mod_api::Command::op("cop", 2).executes_client(|_, _| Some(1)));
                let tagged = self.heard.clone();
                event.commands.push(
                    mod_api::Command::new(
                        "cbiome <biome:resource_or_tag:minecraft:worldgen/biome>",
                    )
                    .executes_client(move |_, invocation| {
                        tagged.lock().unwrap().push(invocation.clone());
                        Some(1)
                    }),
                );
                event.commands.push(
                    mod_api::Command::new("cserver <target:player>")
                        .executes_client(|_, _| Some(1)),
                );
            }
        }

        let heard = Arc::new(Mutex::new(Vec::new()));
        let sibling = Arc::new(Mutex::new(Vec::new()));
        let recursed = Arc::new(AtomicUsize::new(0));
        let ping_pong = Arc::new(Mutex::new(Vec::new()));
        let dropped = Arc::new(AtomicBool::new(false));
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "client_cmd_fixture",
            Box::new(ClientCommandsMod {
                heard: heard.clone(),
                sibling: sibling.clone(),
                recursed: recursed.clone(),
                ping_pong: ping_pong.clone(),
                dropped: dropped.clone(),
            }),
        )]
        .into();

        let _ = mod_api::testing::rebuild_parts(|| -> Result<(), ()> {
            sunrise_gameplay_common::game::api_values::install_parts(&mods).unwrap();
            mod_platform::testing::with_mods(mods, || {
                crate::game::native_entity_refs::tests::with_client_game(|game| {
                    let _ = mod_api::__private::take_hook_panics();
                    let chat_tree = game.chat_tree_for(None);
                    assert!(chat_tree.find("cmach").is_some());
                    assert!(chat_tree.find("cpanic").is_some());
                    assert!(chat_tree.find("csibling").is_some());
                    // An operator-level client usage is in the tree behind its level (#5654).
                    let cop = chat_tree.find("cop").expect("an operator client command");
                    assert_eq!(chat_tree.nodes[cop].permission, 2);
                    assert!(chat_tree.find("cserver").is_none());
                    let tp = chat_tree.find("tp").expect("tp redirect preserved");
                    let tp_target = chat_tree.nodes[tp].redirect.expect("tp redirect preserved");
                    assert_eq!(chat_tree.nodes[tp_target].name(), "teleport");
                    assert_eq!(chat_tree.nodes[tp].permission, 2);

                    // Unregistered or rejected roots return false so they proceed to the server.
                    assert_eq!(game.run_client_native_command("unknown_root foo"), None);
                    // Below its level the local player can't run it; at the level it runs.
                    game.ui.registries.permission = 0;
                    assert_eq!(game.run_client_native_command("cop"), None);
                    game.ui.registries.permission = 2;
                    assert_eq!(
                        game.run_client_native_command("cop"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    game.ui.registries.permission = 0;
                    assert_eq!(game.run_client_native_command("cserver @a"), None);
                    // A resource-or-tag argument tells a `#tag` from an id (#5654).
                    for (text, tag, id) in [
                        ("cbiome #minecraft:is_ocean", true, "minecraft:is_ocean"),
                        ("cbiome minecraft:plains", false, "minecraft:plains"),
                    ] {
                        assert_eq!(
                            game.run_client_native_command(text),
                            Some(ClientCommandOutcome::Success(1)),
                            "{text}"
                        );
                        let invocation = heard.lock().unwrap().pop().unwrap();
                        assert_eq!(
                            invocation.argument("biome"),
                            Some(&mod_api::CommandArgumentValue::ResourceOrTag {
                                tag,
                                id: mod_api::Identifier::parse(id).unwrap(),
                            }),
                            "{text}"
                        );
                    }

                    // Typed client invocation parses custom arguments, coordinates and optionals.
                    assert_eq!(
                        game.run_client_native_command("cmach press 1 2 3 7"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    {
                        let heard = heard.lock().unwrap();
                        assert_eq!(heard.len(), 1);
                        assert_eq!(heard[0].literals, ["cmach"]);
                        assert_eq!(
                            heard[0].argument("machine"),
                            Some(&mod_api::CommandArgumentValue::Custom {
                                kind: "client_cmd_fixture:machine".into(),
                                value: "press".into(),
                            })
                        );
                        assert_eq!(
                            heard[0].argument("pos"),
                            Some(&mod_api::CommandArgumentValue::BlockPos([1, 2, 3]))
                        );
                        assert_eq!(
                            heard[0].argument("count"),
                            Some(&mod_api::CommandArgumentValue::Integer(7))
                        );
                    }

                    // A syntax error on a registered client root reports locally and never calls the executor.
                    let before_err = game.client.chat_ui.chat.recent_lines(50).len();
                    assert_eq!(
                        game.run_client_native_command("cmach nope 1 2 3"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert_eq!(heard.lock().unwrap().len(), 1);
                    assert!(game.client.chat_ui.chat.recent_lines(50).len() > before_err);

                    // A panicking client executor disables only itself, emits a notice, and leaves siblings working.
                    let before_panic = game.client.chat_ui.chat.recent_lines(50).len();
                    assert_eq!(
                        game.run_client_native_command("cpanic"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert!(game.client.chat_ui.chat.recent_lines(50).len() > before_panic);
                    let notices: Vec<_> = mod_api::__private::take_hook_panics()
                        .into_iter()
                        .filter(|notice| notice.mod_id == "client_cmd_fixture")
                        .collect();
                    assert_eq!(notices.len(), 1);
                    assert_eq!(notices[0].hook, "client_command.execute");

                    // Disabled executor is not invoked again, and sibling commands still run.
                    assert_eq!(
                        game.run_client_native_command("cpanic"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert!(
                        mod_api::__private::take_hook_panics()
                            .into_iter()
                            .all(|notice| notice.mod_id != "client_cmd_fixture")
                    );
                    assert_eq!(
                        game.run_client_native_command("csibling ok"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    {
                        let sibling = sibling.lock().unwrap();
                        assert_eq!(sibling.len(), 1);
                        assert_eq!(
                            sibling[0].argument("word"),
                            Some(&mod_api::CommandArgumentValue::String("ok".into()))
                        );
                    }

                    // Rebuilding the client tree invalidates parsed executor keys from the previous generation.
                    let old_key = game
                        .client
                        .native_client_commands
                        .dispatcher
                        .parse("cmach press 1 2 3", &game.ui.registries)
                        .executable()
                        .unwrap()
                        .0
                        .executor
                        .expect("client command binds an executor key");
                    assert!(
                        game.client
                            .native_client_commands
                            .dispatcher
                            .owns_executor(old_key)
                    );
                    let mut server = Dispatcher::new();
                    let shared = server.literal(0, "shared");
                    let remote = server.literal(shared, "server");
                    let target = server.literal(0, "target");
                    let shared_alias = server.literal(0, "shared_alias");
                    server.requires(shared, 2);
                    server.requires(remote, 3);
                    server.requires_execution(remote, 3);
                    server.executes(remote, "remote");
                    server.redirect(remote, target);
                    server.redirect(shared_alias, shared);
                    // Refresh the local usage after a connection rebuild without claiming the shared root.
                    let (client, _server) = sunrise_protocol::connection::Pipe::pair();
                    game.connect_local(client, "CommandsTest").unwrap();
                    game.client.remote.as_mut().unwrap().joined = true;
                    assert!(dropped.load(Ordering::Relaxed));
                    let notices: Vec<_> = mod_api::__private::take_hook_panics()
                        .into_iter()
                        .filter(|notice| notice.mod_id == "client_cmd_fixture")
                        .collect();
                    assert_eq!(notices.len(), 1);
                    assert_eq!(notices[0].hook, "client_command.drop");
                    let display = game.chat_tree_for(Some(&server));
                    let children = &display.nodes[shared].children;
                    assert!(
                        children
                            .iter()
                            .any(|&id| display.nodes[id].name() == "local")
                    );
                    assert!(children.contains(&remote));
                    assert_eq!(display.nodes[shared].permission, 2);
                    assert_eq!(display.nodes[remote].execution_permission, 3);
                    let remote_target = display.nodes[remote]
                        .redirect
                        .expect("server redirect preserved");
                    assert_eq!(display.nodes[remote_target].name(), "target");
                    let op_reg = sunrise_command::args::Registries {
                        permission: 3,
                        ..Default::default()
                    };
                    assert_eq!(
                        display
                            .parse("shared_alias server", &op_reg)
                            .executable()
                            .unwrap()
                            .1,
                        "remote"
                    );
                    assert!(
                        display
                            .parse("shared_alias local 7", &op_reg)
                            .executable()
                            .is_err()
                    );
                    let alias_suggestions: Vec<String> = display
                        .client_suggestions(&display.parse("shared_alias ", &op_reg), 13, &op_reg)
                        .1
                        .into_iter()
                        .map(|s| s.text)
                        .collect();
                    assert_eq!(alias_suggestions, ["server"]);

                    assert_eq!(game.run_client_native_command("shared server"), None);
                    assert_eq!(game.run_client_native_command("shared"), None);
                    assert_eq!(
                        game.run_client_native_command("shared local nope"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert_eq!(
                        game.send_signed_chat_input("/shared local 7"),
                        Some(ClientCommandOutcome::Success(42))
                    );
                    assert_eq!(
                        game.send_signed_chat_input("/shared refused"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert_eq!(
                        game.run_client_native_command("cpanic"),
                        Some(ClientCommandOutcome::Failed)
                    );
                    assert!(
                        mod_api::__private::take_hook_panics()
                            .into_iter()
                            .all(|notice| notice.mod_id != "client_cmd_fixture")
                    );
                    assert_eq!(
                        game.send_signed_chat_input("/csibling signed"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    game.send_unattended_command("/csibling unattended");
                    // Ordinary top-level Host submission intercepts locally without waiting for a frame.
                    assert!(NetworkClientHost::send_chat(game, "/csibling host_top").is_ok());
                    {
                        let sibling = sibling.lock().unwrap();
                        assert_eq!(sibling.len(), 4);
                        assert_eq!(
                            sibling[2].argument("word"),
                            Some(&mod_api::CommandArgumentValue::String("unattended".into()))
                        );
                        assert_eq!(
                            sibling[3].argument("word"),
                            Some(&mod_api::CommandArgumentValue::String("host_top".into()))
                        );
                    }
                    assert!(game.command_prompt_message().is_none());

                    // Submitting the same executor defers the attempt, then refuses it locally.
                    assert!(NetworkClientHost::send_chat(game, "/crecurse").is_ok());
                    assert_eq!(recursed.load(Ordering::Relaxed), 1);
                    let chain = game.client.native_client_commands.pending_commands[0]
                        .chain
                        .clone();
                    game.native_frame_end().unwrap();
                    assert_eq!(recursed.load(Ordering::Relaxed), 1);
                    assert!(
                        game.client
                            .native_client_commands
                            .pending_commands
                            .is_empty()
                    );
                    for (line, kind) in [
                        ("crecurse", PendingClientCommandKind::Signed),
                        ("/crecurse", PendingClientCommandKind::Unattended),
                    ] {
                        assert_eq!(
                            game.submit_client_command(line, kind, Some(chain.clone())),
                            Ok(Some(ClientCommandOutcome::Failed))
                        );
                    }
                    assert!(game.command_prompt_message().is_none());

                    // A tree refresh between frames must preserve each branch’s executor ancestry.
                    assert!(NetworkClientHost::send_chat(game, "/cping").is_ok());
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping"]);
                    game.native_frame_end().unwrap();
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping", "pong", "pong"]);
                    assert_eq!(sibling.lock().unwrap().len(), 4);
                    game.chat_tree_for(Some(&server));
                    let before_refusals = game.client.chat_ui.chat.recent_lines(50).len();
                    game.native_frame_end().unwrap();
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping", "pong", "pong"]);
                    assert_eq!(
                        game.client.chat_ui.chat.recent_lines(50).len(),
                        before_refusals + 4
                    );
                    assert!(
                        game.client
                            .native_client_commands
                            .pending_commands
                            .is_empty()
                    );
                    {
                        let sibling = sibling.lock().unwrap();
                        let nested = Some(mod_api::CommandArgumentValue::String("nested".into()));
                        assert_eq!(sibling.len(), 6);
                        for invocation in &sibling[4..] {
                            assert_eq!(invocation.argument("word"), nested.as_ref());
                        }
                    }
                    // A fresh player submission has no ancestors and still intercepts locally.
                    assert_eq!(
                        game.send_signed_chat_input("/cping"),
                        Some(ClientCommandOutcome::Success(1))
                    );
                    assert_eq!(*ping_pong.lock().unwrap(), ["ping", "pong", "pong", "ping"]);

                    assert!(
                        !game
                            .client
                            .native_client_commands
                            .dispatcher
                            .owns_executor(old_key)
                    );
                });
            });
            Ok(())
        });
    }
}
