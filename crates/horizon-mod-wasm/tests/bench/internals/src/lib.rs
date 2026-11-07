//! Each raw call performs N accesses; the host times and verifies the committed result.
use mod_internals::view::{Game, HandleChatInputRaw};
mod allocations;

#[derive(Default)]
pub struct Mod;
#[mod_api::horizon_mod]
impl mod_api::Hooks for Mod {}

fn access(raw: HandleChatInputRaw, game: Option<Game>) {
    let game = game.unwrap();
    let message = raw.message().unwrap();
    let (mode, count) = message.split_once(' ').unwrap();
    let count: i64 = count.parse().unwrap();
    if mode == "trap" {
        game.set_copy(99).unwrap();
        panic!("discard this listener's pending write");
    }
    let counting = mode.ends_with("-alloc");
    let mode = mode.trim_end_matches("-alloc");
    allocations::start(counting);
    let mut total = 0;
    for i in 0..count {
        match mode {
            "get" => total += game.copy().unwrap(),
            "handle" => total += game.child().unwrap().value().unwrap(),
            "set" => game.set_copy(i).unwrap(),
            "call" => total += game.add(1).unwrap(),
            mode if mode.starts_with("get1") || mode.starts_with("set1") => {
                macro_rules! fields {
                    ($($field:ident:$get:ident:$set:ident),+) => {
                        if mode.starts_with("get") {
                            if mode.ends_with("batch") {
                                let ($($field,)+) = mod_internals::view::get_many(($(game.$field(),)+));
                                $(total += $field.unwrap();)+
                            } else { $(total += game.$get().unwrap();)+ }
                        } else if mode.ends_with("batch") {
                            let ($($field,)+) = mod_internals::view::set_many(($(game.$field().write(i),)+));
                            $($field.unwrap();)+
                        } else { $(game.$set(i).unwrap();)+ }
                    };
                }
                if mode.contains("10") {
                    fields!(f0_field:f0:set_f0, f1_field:f1:set_f1, f2_field:f2:set_f2, f3_field:f3:set_f3, f4_field:f4:set_f4, f5_field:f5:set_f5, f6_field:f6:set_f6, f7_field:f7:set_f7, f8_field:f8:set_f8, f9_field:f9:set_f9);
                } else {
                    fields!(f0_field:f0:set_f0, f1_field:f1:set_f1, f2_field:f2:set_f2, f3_field:f3:set_f3, f4_field:f4:set_f4, f5_field:f5:set_f5, f6_field:f6:set_f6, f7_field:f7:set_f7, f8_field:f8:set_f8, f9_field:f9:set_f9, f10_field:f10:set_f10, f11_field:f11:set_f11);
                }
            }
            _ => unreachable!(),
        }
    }
    let counts = allocations::stop();
    let result = if counting {
        format!("{} {} {}", counts.0, counts.1, counts.2)
    } else {
        total.to_string()
    };
    raw.set_message(Box::new(result)).unwrap();
}
mod_internals::raw_listeners!();
