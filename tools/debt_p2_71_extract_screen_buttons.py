#!/usr/bin/env python3
"""Lift the existing window button routes out of the keyboard/scroll/drag match."""
from pathlib import Path
from textwrap import dedent, indent


path = Path(__file__).resolve().parents[1] / "crates/sunrise-gameplay-common/src/game.rs"
source = path.read_text()
start = '                WindowEvent::MouseInput { state, button, .. } if self.command_prompt.is_some() => {'
end = '                // ConfirmScreen.keyPressed: Escape answers no, once its delay is over.'
anchor = '    /// One window event, in the order vanilla\'s `KeyboardHandler` and `MouseHandler` see it.'
for marker in (start, end, anchor):
    assert source.count(marker) == 1, marker
first = source.index(start)
last = source.index(end, first)
routes = source[first:last]
assert routes.count('WindowEvent::MouseInput {') == 16
source = (
    source[:first]
    + '                event @ WindowEvent::MouseInput { .. } => {\n'
    + '                    self.route_screen_button(event_loop, event)?;\n'
    + '                }\n'
    + source[last:]
)
helper = (
    '    fn route_screen_button(&mut self, event_loop: &dyn EventLoopCtl, event: WindowEvent) -> Result<()> {\n'
    '        match event {\n'
    + indent(dedent(routes), '            ')
    + '            _ => {}\n'
    '        }\n'
    '        Ok(())\n'
    '    }\n\n'
)
path.write_text(source.replace(anchor, helper + anchor, 1))
