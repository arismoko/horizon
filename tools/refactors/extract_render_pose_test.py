"""Move the render pose regression into its own included fixture; behavior unchanged."""
from pathlib import Path

source = Path('crates/horizon-client/src/mobs/render.rs')
text = source.read_text()
start = text.rfind('    #[test]', 0, text.index('    fn posed_parts_are_the_poses_the_draw_made_and_change_nothing'))
end = text.index('    /// A part `render_player_pre`', start)
fixture = text[start:end]
Path('crates/horizon-client/src/mobs/render_pose_tests.rs').write_text(fixture)
source.write_text(text[:start] + '    include!("render_pose_tests.rs");\n\n' + text[end:])
