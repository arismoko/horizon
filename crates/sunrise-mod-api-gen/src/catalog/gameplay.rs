use super::{DispatchStyle, Hook, PhaseContext, Receiver, Side};
use crate::GameplayPackage;
use syn::{Path, Type};

pub(crate) struct Binding {
    pub package: GameplayPackage,
    pub module: String,
    pub function: String,
    pub kind: BindingKind,
}

pub(crate) enum BindingKind {
    Shared,
    Nominal,
    Scoped(syn::PathSegment),
    Free,
    Borrowed(Type),
}

impl BindingKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Shared => "hook_shared",
            Self::Nominal => "hook_nominal",
            Self::Scoped(_) => "hook_scoped",
            Self::Free => "hook_free",
            Self::Borrowed(_) => "hook_borrowed",
        }
    }

    pub(crate) fn level(&self) -> bool {
        let owner = match self {
            Self::Scoped(owner) => Some(owner),
            Self::Borrowed(Type::Path(ty)) => ty.path.segments.last(),
            _ => None,
        };
        owner.is_some_and(|owner| owner.ident == "GameLevel" || owner.ident == "LevelView")
    }
}

impl Hook {
    pub(crate) fn gameplay_binding(&self) -> Result<Option<Binding>, String> {
        let invalid = |why: &str| format!("{}: {why} ({})", self.name, self.target);
        let Some(package) = self
            .target
            .split("::")
            .next()
            .and_then(GameplayPackage::from_library)
        else {
            return if self.receiver_type.is_none() {
                Ok(None)
            } else {
                Err(invalid("receiver_type requires a gameplay free function"))
            };
        };
        let path: Path =
            syn::parse_str(&self.target).map_err(|error| invalid(&error.to_string()))?;
        let mut segments = path.segments.into_iter().collect::<Vec<_>>();
        let function = segments
            .pop()
            .ok_or_else(|| invalid("missing binding function"))?
            .ident
            .to_string();
        if function != format!("native_{}", self.name)
            || segments.len() < 3
            || segments[1].ident != "game"
            || segments[2].ident != "native_hooks"
        {
            return Err(invalid(
                "expected a native hook in the defining gameplay module",
            ));
        }
        let owner = segments
            .last()
            .filter(|segment| {
                segment
                    .ident
                    .to_string()
                    .starts_with(|ch: char| ch.is_ascii_uppercase())
                    || !segment.arguments.is_empty()
            })
            .cloned();
        if owner.is_some() {
            segments.pop();
        }
        let module = segments
            .iter()
            .skip(1)
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        let kind = match (owner, self.receiver, &self.receiver_type) {
            (None, Receiver::Associated, None) => BindingKind::Free,
            (None, Receiver::Read | Receiver::Write, Some(ty))
                if self.dispatch != DispatchStyle::Direct =>
            {
                let ty: Type = syn::parse_str(ty).map_err(|error| invalid(&error.to_string()))?;
                if !matches!(&ty, Type::Path(path) if path.qself.is_none()) {
                    return Err(invalid("receiver_type must name the referent type"));
                }
                BindingKind::Borrowed(ty)
            }
            (Some(owner), Receiver::Read | Receiver::Write, None) if owner.ident == "GameHooks" => {
                if package != GameplayPackage::Common || self.side != Side::Both {
                    return Err(invalid(
                        "GameHooks requires common gameplay and a both-side hook",
                    ));
                }
                BindingKind::Shared
            }
            (Some(owner), Receiver::Read | Receiver::Write, None)
                if owner.ident == "ServerGame" || owner.ident == "ClientGame" =>
            {
                if package == GameplayPackage::Common || owner.ident != package.game_type() {
                    return Err(invalid(
                        "nominal receiver does not belong to the binding package",
                    ));
                }
                BindingKind::Nominal
            }
            (Some(owner), Receiver::Read | Receiver::Write, None)
                if owner.ident != "Game" && owner.ident != "CommonGame" =>
            {
                BindingKind::Scoped(owner)
            }
            _ => return Err(invalid("binding and declared receiver form disagree")),
        };
        if package != GameplayPackage::Common && package != GameplayPackage::for_side(self.side) {
            return Err(invalid("binding package does not match the hook side"));
        }
        if package == GameplayPackage::Common
            && matches!(
                self.context,
                Some(
                    PhaseContext::ClientReload
                        | PhaseContext::ClientReloadListener
                        | PhaseContext::ClientSetup
                        | PhaseContext::ClientIdentity
                )
            )
        {
            return Err(invalid("client phase cannot live in common gameplay"));
        }
        Ok(Some(Binding {
            package,
            module,
            function,
            kind,
        }))
    }

    pub(crate) fn apply_gameplay_default(&mut self) {
        let Some(module) = &self.default_module else {
            return;
        };
        let package = GameplayPackage::for_side(self.side);
        let library = package.domain().key;
        if self.receiver == Receiver::Associated {
            self.target = format!(
                "{library}::game::native_hooks::{module}::native_{}",
                self.name
            );
        } else {
            let owner = match package {
                GameplayPackage::Common => "GameHooks",
                GameplayPackage::Server => "ServerGame<'_>",
                GameplayPackage::Client => "ClientGame<'_>",
            };
            self.target = format!(
                "{library}::game::native_hooks::{module}::{owner}::native_{}",
                self.name
            );
        }
    }
}
