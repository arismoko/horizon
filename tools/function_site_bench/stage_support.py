"""Corrections needed when transplanting current staging onto the fixed kit base."""


def exclude_generated_point_helpers(stage):
    """Do not rediscover replacement installers as public game injection points."""
    path = stage / 'crates/horizon-mod-point-shape/src/lib.rs'
    source = path.read_text()
    old = '    !sig.ident.to_string().starts_with("native_")'
    assert source.count(old) == 1
    source = source.replace(old, '    let name = sig.ident.to_string();\n    !name.starts_with("native_")\n        && !name.starts_with("__fnr_")', 1)
    path.write_text(source)
