use crate::game::server_side::commands::summon_item_orb_tests::with_game;

pub(crate) fn run_summon_replies_with_the_custom_name_of_every_kind() {
    let path = std::env::var_os("SUNRISE_SUMMON_CUSTOM_NAME_WITNESS").unwrap_or_else(|| {
        panic!(
            "SUNRISE_SUMMON_CUSTOM_NAME_WITNESS is unset; run tools/test_summon_custom_name_parity.py"
        )
    });
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "failed to read witness {} (record via tools/test_summon_custom_name_parity.py): {e}",
            std::path::Path::new(&path).display()
        )
    });
    let witness: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(witness["minecraft"], "26.3");
    assert_eq!(
        witness["oracle"],
        "dev.sunriselab.parity.VanillaCommandExecOracle"
    );
    assert_eq!(
        witness["groups"],
        serde_json::json!(["summon custom name replies"])
    );
    let records = witness["records"].as_array().unwrap();

    let mut expected_cases: Vec<(String, bool)> = vec![
        (
            "/execute positioned 4.5 20000000 4.5 summon evoker_fangs run tag @s add out_of_bounds"
                .into(),
            true,
        ),
        ("/execute if entity @e[type=evoker_fangs]".into(), false),
        (
            "/execute if entity @e[type=evoker_fangs,tag=out_of_bounds]".into(),
            false,
        ),
        (
            "/execute positioned 4.5 200 4.5 summon evoker_fangs run tag @s add in_bounds".into(),
            false,
        ),
        (
            "/execute if entity @e[type=evoker_fangs,tag=in_bounds]".into(),
            false,
        ),
        ("/team add t2694".into(), false),
        ("/team modify t2694 color red".into(), false),
        ("/team modify t2694 prefix \"[R] \"".into(), false),
        ("/team modify t2694 suffix \"!\"".into(), false),
    ];
    let kinds = [
        (1u32, "evoker_fangs", "2.5", ""),
        (2, "experience_orb", "5.5", ""),
        (3, "area_effect_cloud", "8.5", ""),
        (4, "ender_dragon", "11.5", ""),
        (5, "pig", "14.5", "NoAI:1b,NoGravity:1b,"),
    ];
    let variants = [
        (1u32, ""),
        (2, "CustomName:\"\""),
        (3, "CustomName:\"fz\""),
        (4, "CustomName:{text:\"fz\",color:\"gold\",bold:true}"),
        (
            5,
            "CustomName:{text:\"fz\",color:\"gold\",click_event:{action:\"run_command\",command:\"/say hi\"},extra:[{text:\"!\",color:\"aqua\",click_event:{action:\"suggest_command\",command:\"/help\"}}]}",
        ),
        (
            6,
            "CustomName:{text:\"fz\",hover_event:{action:\"show_text\",value:\"old\"},insertion:\"old\"}",
        ),
        (7, "CustomName:42"),
        (8, "CustomName:{}"),
        (9, "CustomName:{text:42}"),
        (10, "CustomName:[]"),
    ];
    for (kind_id, kind, x, extra) in kinds {
        for (case_id, name_nbt) in variants {
            let z = 4.5 + f64::from(case_id % 5) * 2.0;
            let suffix = if name_nbt.is_empty() {
                String::new()
            } else {
                format!(",{name_nbt}")
            };
            expected_cases.push((
                format!(
                    "/summon {kind} {x} 200 {z} {{{extra}UUID:[I;2694,0,{kind_id},{case_id}]{suffix}}}"
                ),
                true,
            ));
            if kind == "ender_dragon" {
                expected_cases.push((
                    format!("/kill 00000a86-0000-0000-0000-{kind_id:04x}{case_id:08x}"),
                    true,
                ));
            }
        }
        expected_cases.push((
            format!(
                "/summon {kind} {x} 200 14.5 {{{extra}UUID:[I;2694,1,{kind_id},1],CustomName:\"fz\",Team:\"t2694\",Tags:[\"t2694_team\"]}}"
            ),
            true,
        ));
        expected_cases.push((
            format!("/execute if entity @e[type={kind},tag=t2694_team,name=fz]"),
            false,
        ));
        expected_cases.push((
            format!("/execute if entity @e[type={kind},tag=t2694_team,name=\"[R] fz!\"]"),
            false,
        ));
        if kind == "ender_dragon" {
            expected_cases.push((
                format!("/kill 00000a86-0000-0001-0000-{kind_id:04x}00000001"),
                true,
            ));
        }
    }
    assert_eq!(
        records.len(),
        expected_cases.len(),
        "witness record count differs from expected cases"
    );

    with_game(|game| {
        for (index, (record, (expected_cmd, expect_components))) in
            records.iter().zip(expected_cases.iter()).enumerate()
        {
            let cmd = record["cmd"].as_str().unwrap();
            assert_eq!(cmd, expected_cmd.as_str(), "case #{index} identity/order");
            let line = cmd.strip_prefix('/').unwrap_or(cmd);
            game.server.chat_capture = Some(Vec::new());
            game.server.cmds.0.results = Some(Vec::new());
            game.run_command(line);
            let captured = game.server.chat_capture.take().unwrap_or_default();
            let expected_messages = record["messages"].as_array().unwrap();
            assert_eq!(
                captured.len(),
                expected_messages.len(),
                "case #{index} message count: {cmd}"
            );
            let messages: Vec<serde_json::Value> = captured
                .iter()
                .map(sunrise_gameplay::game::server_side::command_harness::spans)
                .collect();
            assert_eq!(
                serde_json::Value::Array(messages),
                record["messages"],
                "case #{index} spans: {cmd}"
            );
            if *expect_components {
                let expected_components = record["components"]
                    .as_array()
                    .expect("structured components recorded");
                assert_eq!(
                    captured.len(),
                    expected_components.len(),
                    "case #{index} component count: {cmd}"
                );
                let components: Vec<serde_json::Value> = captured
                    .iter()
                    .map(sunrise_gameplay::game::server_side::command_harness::canonical_component)
                    .collect();
                assert_eq!(
                    serde_json::Value::Array(components),
                    record["components"],
                    "case #{index} components: {cmd}"
                );
            } else {
                assert!(
                    record.get("components").is_none(),
                    "case #{index} unexpected components field: {cmd}"
                );
            }
            let results: Vec<serde_json::Value> = game
                .server
                .cmds
                .0
                .results
                .take()
                .unwrap_or_default()
                .into_iter()
                .map(|(success, value)| serde_json::json!([success, value]))
                .collect();
            assert_eq!(
                serde_json::Value::Array(results),
                record["results"],
                "case #{index} results: {cmd}"
            );
        }
    });
}

/// `ItemEntity(EntityType, Level)` leaves the stack `ItemStack.EMPTY`, and
/// `readAdditionalSaveData` discards such an entity, so `addEntity` adds
/// nothing; `tryAddFreshEntityWithPassengers` still answers true, so both
/// summon forms report success. The entity id is taken all the same, and the
/// name is `ItemStack.EMPTY.getItemName()`: an empty component.
#[test]
#[ignore = "daily"]
fn summon_item_succeeds_with_and_without_nbt_without_leaving_an_entity() {
    with_game(|game| {
        for command in [
            "summon minecraft:item 4.5 200 4.5",
            "summon minecraft:item 4.5 200 4.5 {}",
        ] {
            let before = game.server.next_entity;
            game.server.chat_capture = Some(Vec::new());
            let ok = game.run_command(command);
            let lines = game.server.chat_capture.take().unwrap_or_default();
            assert!(ok, "{command}: {lines:?}");
            assert_eq!(game.server.next_entity, before + 1, "{command}");
            assert!(game.dropped.is_empty(), "{command}");
            let components: Vec<serde_json::Value> = lines
                .iter()
                .map(|line| {
                    serde_json::to_value(line.component()).expect("text component serializes")
                })
                .collect();
            assert_eq!(
                components,
                vec![serde_json::json!({
                    "translate": "commands.summon.success",
                    "with": [{ "text": "" }],
                })],
                "{command}"
            );
        }
    });
}
