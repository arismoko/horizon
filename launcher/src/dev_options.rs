//! Developer options: a Settings section nothing in the window points to. Seven clicks on the
//! sidebar's "Launcher x.y.z" reveal it for this run; it stays shown while one of its options is
//! on. One option lets offline names play without an account that owns the game
//! (`backend::Settings::dev_offline`). The game still comes from the player's own install: the
//! launcher downloads Mojang's files only for an owner (`backend::download_for_owner`). The other
//! builds native mods with the dev profile (`backend::Settings::native_profile`), for developing
//! one; players keep `performance`.
use super::*;
use native_mods::Profile;

/// Clicks on the launcher's version that reveal the section.
const CLICKS: u8 = 7;

impl Launcher {
    /// A click on the sidebar's launcher version; the last one opens Settings on the section.
    pub(super) fn version_clicked(&mut self) {
        if self.dev_options {
            return;
        }
        self.dev_clicks += 1;
        if self.dev_clicks >= CLICKS {
            self.dev_options = true;
            self.go(Page::Settings);
        }
    }

    /// The section, when revealed or when one of its options is on.
    pub(super) fn dev_options_section(&self, cx: &mut Context<Self>) -> Option<Div> {
        let dev_builds = self.settings.native_profile == Some(Profile::Dev);
        if !self.dev_options && !self.settings.dev_offline && !dev_builds {
            return None;
        }
        let t = self.theme;
        let toggle = switch("dev-offline", self.settings.dev_offline, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.dev_offline = !this.settings.dev_offline;
                this.settings.save();
                cx.notify();
            },
        ));
        let dev_toggle = switch("dev-profile", dev_builds, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                let on = this.settings.native_profile != Some(Profile::Dev);
                set_dev_builds(&mut this.settings, on);
                this.native_done = None;
                this.settings.save();
                cx.notify();
            },
        ));
        Some(
            div()
                .flex()
                .flex_col()
                .child(
                    section_title("Developer options", "terminal", &t)
                        .mt(px(20.))
                        .mb(px(12.)),
                )
                .child(index_row(
                    0,
                    tr("Offline names without an account"),
                    Some(
                        div()
                            .child(tr(
                                "Offline names play without a Microsoft account that owns the game. \
                                 Minecraft still comes from your own install.",
                            ))
                            .into_any_element(),
                    ),
                    Some(toggle.into_any_element()),
                    &t,
                ))
                .child(index_row(
                    1,
                    tr("Faster mod builds (dev profile)"),
                    Some(
                        div()
                            .child(tr(
                                "Mod edits rebuild about 6× faster; the game runs slower.",
                            ))
                            .into_any_element(),
                    ),
                    Some(dev_toggle.into_any_element()),
                    &t,
                )),
        )
    }
}

/// The developer option's switch: `dev` on, back to the default (`performance`) off.
fn set_dev_builds(settings: &mut backend::Settings, on: bool) {
    settings.native_profile = on.then_some(Profile::Dev);
    settings.native_release = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dev_builds_switch_writes_the_profile_setting() {
        let mut settings = backend::Settings {
            native_release: true,
            ..Default::default()
        };
        set_dev_builds(&mut settings, true);
        assert_eq!(Profile::chosen(&settings), Profile::Dev);
        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("\"native_profile\":\"dev\""), "{text}");
        set_dev_builds(&mut settings, false);
        assert_eq!(Profile::chosen(&settings), Profile::Performance);
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("native_profile"));
    }
}
