use horizon_core::nbt::{Compound, JavaString, Tag};
use horizon_items::items::ItemStack as EngineStack;
use mod_api::*;
use std::{
    hint::black_box,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

#[cfg(feature = "alloc-count")]
mod allocations {
    use std::{
        alloc::{GlobalAlloc, Layout, System},
        cell::Cell,
    };
    thread_local! { static COUNT: Cell<Option<(usize, usize)>> = const { Cell::new(None) }; }
    struct Allocator;
    #[global_allocator]
    static ALLOCATOR: Allocator = Allocator;
    fn count(bytes: usize) {
        let _ = COUNT.try_with(|c| {
            if let Some((n, b)) = c.get() {
                c.set(Some((n + 1, b + bytes)));
            }
        });
    }
    unsafe impl GlobalAlloc for Allocator {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 {
            count(l.size());
            unsafe { System.alloc(l) }
        }
        unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
            count(l.size());
            unsafe { System.alloc_zeroed(l) }
        }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
            count(n);
            unsafe { System.realloc(p, l, n) }
        }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
            unsafe { System.dealloc(p, l) }
        }
    }
    pub fn measure(mut f: impl FnMut()) -> (usize, usize) {
        COUNT.set(Some((0, 0)));
        for _ in 0..100 {
            f();
        }
        COUNT.replace(None).unwrap()
    }
}

mod game {
    use super::*;
    use horizon_items::items::ItemStack;
    #[derive(Default)]
    pub struct Game<'a>(std::marker::PhantomData<&'a ()>);
    impl mod_api::Host for Game<'_> {}
    impl mod_api::ServerHost for Game<'_> {}
    mod_api::impl_host_areas!(Game<'_>);
    mod_api::impl_server_host_areas!(Game<'_>);
    include!(concat!(env!("OUT_DIR"), "/bench_fire.rs"));
    // COPIED_CONVERTER
}

static CALLS: AtomicUsize = AtomicUsize::new(0);
struct Part;
#[mod_api::horizon_part]
impl ItemPart for Part {
    fn use_item(&self, _: &mut dyn ServerHost, event: &mut UseItemEvent<'_>) -> Flow {
        black_box(event.stack);
        CALLS.fetch_add(1, Ordering::Relaxed);
        Flow::Continue
    }
}
struct BenchMod(bool);
#[mod_api::__subscriptions]
impl Hooks for BenchMod {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        if self.0 {
            for id in ["minecraft:stick", "minecraft:iron_sword"] {
                let key = r.item_key(id);
                r.item_part(key, Part);
            }
        }
    }
}
fn install(subscribed: bool) {
    let mods: Arc<[ModEntry]> =
        vec![ModEntry::new("stack-bench", Box::new(BenchMod(subscribed)))].into();
    mod_api::__private::install(mods.clone());
    mod_api::__private::install_parts(&mods, &mod_api::__private::part_tests::rule_schema())
        .unwrap();
    // RESOLVE_ITEMS
    mod_api::__private::mark_game_thread();
    mod_api::__private::resolve_parts(
        &Default::default(),
        &Default::default(),
        mod_api::__private::KnownIds {
            block: &|_| false,
            item: &|id| ["minecraft:stick", "minecraft:iron_sword"].contains(&id),
            state_count: 0,
            state_block: &|_| unreachable!(),
        },
    )
    .unwrap();
}
fn report(label: &str, mut f: impl FnMut()) {
    for _ in 0..2_000 {
        f();
    }
    #[cfg(not(feature = "alloc-count"))]
    {
        let mut samples: Vec<f64> = (0..9)
            .map(|_| {
                let start = Instant::now();
                for _ in 0..200_000 {
                    f();
                }
                start.elapsed().as_nanos() as f64 / 200_000.0
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        println!("TIME,{label},{:.3}", samples[4]);
    }
    #[cfg(feature = "alloc-count")]
    {
        let (n, bytes) = allocations::measure(f);
        println!(
            "ALLOC,{label},{:.3},{:.3}",
            n as f64 / 100.0,
            bytes as f64 / 100.0
        );
    }
}
fn with_patch(base: &EngineStack, patch: Compound) -> EngineStack {
    let (_, extra) = horizon_items::item_data::decode_components(Some(&Tag::Compound(patch)));
    EngineStack {
        extra,
        ..base.clone()
    }
}
fn main() {
    install(true);
    let plain = EngineStack::new("minecraft:stick", 1);
    let named = with_patch(
        &plain,
        [(
            JavaString::from("minecraft:custom_name"),
            Tag::String(JavaString::from("Bench stack")),
        )]
        .into(),
    );
    let nested = with_patch(
        &plain,
        [(
            JavaString::from("minecraft:custom_data"),
            Tag::Compound(
                [
                    (
                        JavaString::from("values"),
                        Tag::List((0..32).map(Tag::Int).collect()),
                    ),
                    (JavaString::from("bytes"), Tag::ByteArray(vec![7; 128])),
                    (
                        JavaString::from("label"),
                        Tag::String(JavaString::from("Nested NBT")),
                    ),
                ]
                .into(),
            ),
        )]
        .into(),
    );
    let mut damaged = EngineStack::new("minecraft:iron_sword", 1);
    damaged.damage = 5;
    let mut game = game::Game::default();
    for (name, stack) in [
        ("plain", &plain),
        ("named", &named),
        ("nbt_vec", &nested),
        ("damage", &damaged),
    ] {
        let before = CALLS.load(Ordering::Relaxed);
        let mut claimed = false;
        game.fire_use_item(stack, &mut claimed);
        assert_eq!(CALLS.load(Ordering::Relaxed), before + 1);
        assert!(!claimed);
        report(&format!("use_item/{name}"), || {
            let mut claimed = false;
            game.fire_use_item(black_box(stack), &mut claimed);
            black_box(claimed);
        });
        // SNAPSHOT_REPORT
    }
    // API_REPORT
    install(false);
    let before = CALLS.load(Ordering::Relaxed);
    report("use_item/unsubscribed_named", || {
        let mut claimed = false;
        game.fire_use_item(black_box(&named), &mut claimed);
        black_box(claimed);
    });
    assert_eq!(CALLS.load(Ordering::Relaxed), before);
    println!("SIZE,engine_stack,{}", std::mem::size_of::<EngineStack>());
    println!(
        "SIZE,api_stack,{}",
        std::mem::size_of::<mod_api::ItemStack>()
    );
    println!("SIZE,item_key,{}", std::mem::size_of::<ItemKey>());
    // RECORD_SIZE
}
