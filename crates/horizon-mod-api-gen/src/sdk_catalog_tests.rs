use super::*;

#[test]
fn schema_order_refusals_and_drift() {
    let hooks = vec![
        json!({"id":"z", "arguments":[{"name":"second","type":"u32"},{"name":"first","type":"bool"}],"output":{"type":"u64"}}),
        json!({"id":"a"}),
    ];
    let points = vec![
        json!({"name":"same","target":"z","access":["api"]}),
        json!({"name":"same","target":"a","access":["internals"]}),
        json!({"name":"refused","target":"b","api_refusal":"missing type Damage"}),
    ];
    let original = serialize("7", hooks.clone(), points.clone()).unwrap();
    assert_eq!(
        original,
        serialize("7", hooks.clone(), points.clone()).unwrap()
    );
    let document: Value = serde_json::from_str(&original).unwrap();
    let consume = |value: &Value| -> Result<(), &str> {
        if value["schema_version"] != 1 {
            return Err("unsupported schema");
        }
        Ok(())
    };
    assert_eq!(consume(&document), Ok(()));
    assert_eq!(document["api_version"], "7");
    assert_eq!(document["hooks"].as_array().unwrap().len(), 2);
    assert_eq!(document["points"].as_array().unwrap().len(), 3);
    assert_eq!(document["points"][0]["api_refusal"], "missing type Damage");
    assert_eq!(document["hooks"][1]["arguments"][0]["name"], "second");
    assert_eq!(document["hooks"][1]["arguments"][1]["type"], "bool");
    assert_eq!(document["hooks"][1]["output"]["type"], "u64");
    assert_eq!(document["points"][1]["target"], "a");
    let mut changed = hooks;
    changed.push(json!({"id":"new"}));
    let changed = serialize("7", changed, points).unwrap();
    assert_ne!(original, changed);
    assert_eq!(
        serde_json::from_str::<Value>(&changed).unwrap()["hooks"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    let mut unknown = document;
    unknown["schema_version"] = json!(2);
    assert_eq!(consume(&unknown), Err("unsupported schema"));
}
