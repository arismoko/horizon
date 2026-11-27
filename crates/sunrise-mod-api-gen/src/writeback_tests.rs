use super::*;

// Compile the generated crossing and the actual handwritten adapter bodies against
// an isolated strict stack boundary, so refusal and atomic writeback are observable.
#[test]
#[ignore = "daily"]
fn untouched_writeback_runtime_and_mutants() {
    let ty: Type = syn::parse_quote!(&mut ItemStack);
    let sibling: Type = syn::parse_quote!(&mut bool);
    let crossing = stack_crossing(
        &[("stack".into(), &ty), ("handled".into(), &sibling)],
        None,
        "Proof",
        false,
        &Default::default(),
    )
    .unwrap();
    assert!(
        crossing
            .after
            .contains("mod proof write refused: {error:?}")
    );
    let generated = format!(
        "{} self.native_generated(&mut __api_stack, &mut __api_handled); {}",
        crossing.before, crossing.after
    );
    let mut methods =
        format!("fn generated(&mut self, stack: &mut Stack, handled: &mut bool) {{ {generated} }}");
    for (name, source, adapter) in [
        (
            "menu_button",
            include_str!("../../sunrise-gameplay-common/src/game/native_hooks/shared.rs"),
            "fn apply_menu_button_cursor(",
        ),
        (
            "menu_quick_move",
            include_str!("../../sunrise-gameplay/src/game/modding/native_quick_move.rs"),
            "fn project_quick_move_remainder(",
        ),
    ] {
        let start = source.find(adapter).unwrap();
        let text = &source[start..];
        let begin = text.find("{").unwrap();
        let mut depth = 1;
        let end = text[begin + 1..]
            .char_indices()
            .find_map(|(i, c)| {
                match c {
                    '{' => depth += 1,
                    '}' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(begin + 1 + i)
            })
            .unwrap();
        let body = &text[begin + 1..end];
        let body = if name == "menu_button" {
            // Keep the adapter's early return inside its unit-returning block.
            format!(
                "let carried = stack; let original = carried.clone(); let mut cursor = original.clone();
                 if self.change {{ cursor = Some(Stack(252)); }}
                 let items = self.items(); let (cursor, original) = (&cursor, &original);
                 (|| {{ {body} }})(); true"
            )
        } else {
            format!(
                "let original = stack.clone(); let mut held = original.clone();
                 if self.change {{ held = Some(Stack(252)); }} let clicked = original.clone().unwrap();
                 let project = |clicked: &Stack, held: &Option<Stack>, original: &Option<Stack>| {{ {body} }};
                 if let Ok(remainder) = project(&clicked, &held, &original) {{
                     *stack = remainder; *handled = true; true
                 }} else {{ false }}"
            )
        };
        methods.push_str(&format!("fn {name}(&mut self, stack: &mut Option<Stack>, handled: &mut bool) -> bool {{ {body} }}"));
    }
    let fixture = r#"
#![allow(unused)]
extern crate self as log;
#[macro_export] macro_rules! warn { ($($arg:tt)*) => { eprintln!($($arg)*); } }
#[derive(Clone, Debug, PartialEq)] struct Stack(u32);
type EngineStack = Stack;
struct Game { change: bool }
enum QuickMoveOutcome { Rejected }
static VALIDATIONS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
mod mod_api {
    #[derive(Debug)] pub enum StackWriteError { Unavailable }
    pub enum HookId { Proof, MenuButton }
    pub mod __private {
        pub fn listened(_: super::HookId) -> bool { true }
        pub fn has_part_hook(_: super::HookId) -> bool { true }
    }
}
mod game { pub mod api_values {
    pub trait TryIntoApi { type Api; fn try_into_api(&self) -> Result<Self::Api, ()>; }
    pub trait TryIntoEngine { type Engine; fn try_into_engine(&self, _: &()) -> Result<Self::Engine, crate::mod_api::StackWriteError>; }
    pub trait ItemBoundary { fn boundary_items(&self) -> Option<&()> { Some(&()) } }
} }
use game::api_values::*;
impl TryIntoApi for Stack { type Api = Stack; fn try_into_api(&self) -> Result<Stack, ()> { Ok(self.clone()) } }
impl TryIntoEngine for Stack { type Engine = Stack; fn try_into_engine(&self, _: &()) -> Result<Stack, mod_api::StackWriteError> { VALIDATIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); if self.0 > 250 { Err(mod_api::StackWriteError::Unavailable) } else { Ok(self.clone()) } } }
impl TryIntoApi for Option<Stack> { type Api = Self; fn try_into_api(&self) -> Result<Self, ()> { Ok(self.clone()) } }
impl TryIntoEngine for Option<Stack> { type Engine = Self; fn try_into_engine(&self, items: &()) -> Result<Self, mod_api::StackWriteError> { self.as_ref().map(|s| s.try_into_engine(items)).transpose() } }
impl ItemBoundary for Game {}
impl Game {
    fn items(&self) -> &() { &() }
    fn native_generated(&mut self, stack: &mut Stack, handled: &mut bool) { if self.change { stack.0 = 252; } *handled = true; }
    METHODS
}
fn main() {
    for change in [false, true] {
        let mut game = Game { change };
        let mut stack = Stack(251);
        let mut handled = false;
        game.generated(&mut stack, &mut handled);
        assert_eq!(stack, Stack(251));
        assert_eq!(handled, !change, "untouched argument must not refuse sibling output");
        type Adapter = fn(&mut Game, &mut Option<Stack>, &mut bool) -> bool;
        let adapters: [(&str, Adapter); 2] = [
            ("menu_button", Game::menu_button),
            ("menu_quick_move", Game::menu_quick_move),
        ];
        for (name, fire) in adapters {
            let mut stack = Some(Stack(251));
            let mut handled = false;
            VALIDATIONS.store(0, std::sync::atomic::Ordering::Relaxed);
            let valid = fire(&mut game, &mut stack, &mut handled);
            assert_eq!(VALIDATIONS.load(std::sync::atomic::Ordering::Relaxed), usize::from(change), "{name}");
            assert_eq!(stack, Some(Stack(251)), "{name}");
            if name == "menu_quick_move" {
                assert_eq!(valid, !change, "{name}");
                assert_eq!(handled, !change, "{name}");
            }
        }
    }
}
"#;
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/untouched-writeback");
    std::fs::create_dir_all(&dir).unwrap();
    let mut variants = vec![
        ("baseline", methods.clone(), true),
        (
            "revalidate",
            methods
                .replace("__api_stack != __original_stack", "true")
                .replace("if cursor == original", "if false")
                .replace("if held == original", "if false"),
            false,
        ),
        (
            "accept_changed",
            methods.replace("__api_stack != __original_stack", "false"),
            false,
        ),
    ];
    for (name, condition) in [
        ("menu_button", "cursor == original"),
        ("menu_quick_move", "held == original"),
    ] {
        let start = methods.find(&format!("fn {name}(")).unwrap();
        let end = methods[start + 3..]
            .find("fn ")
            .map_or(methods.len(), |i| start + 3 + i);
        let mut mutant = methods.clone();
        mutant.replace_range(start..end, &methods[start..end].replace(condition, "false"));
        assert_ne!(mutant, methods);
        variants.push((name, mutant, false));
    }
    for (name, methods, success) in variants {
        let source = dir.join(format!("{name}.rs"));
        let binary = dir.join(name);
        std::fs::write(&source, fixture.replace("METHODS", &methods)).unwrap();
        let output = std::process::Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "rustc could not compile the writeback fixture\n{}\nvariant: {name}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = std::process::Command::new(binary).output().unwrap();
        if name == "baseline" {
            let stderr = String::from_utf8_lossy(&output.stderr);
            for hook in ["proof", "menu_button", "menu_quick_move"] {
                let expected = format!("mod {hook} write refused: Unavailable");
                assert_eq!(stderr.matches(&expected).count(), 1, "{stderr}");
            }
        }
        assert_eq!(
            output.status.success(),
            success,
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
