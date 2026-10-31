fn sample_tick(game: &mut Game<'_>, pos: [i32; 3], entity: &mut ModBlockEntity) {
    use horizon_world::nbt::Tag;
    if entity.id == super::machine_sample::MACHINE {
        let result = entity
            .items
            .first()
            .and_then(Option::as_ref)
            .and_then(|input| {
                game.native_recipes_of_type("machine_sample:processing")
                    .iter()
                    .find_map(|recipe| {
                        let ingredient =
                            game.native_recipe_ingredient(&recipe.json["ingredient"])?;
                        if !ingredient.test(input) {
                            return None;
                        }
                        game.native_recipe_result(&recipe.json["result"])
                    })
            });
        let result = result.filter(|result| {
            let limit = game
                .items()
                .get(&result.item)
                .map_or(0, |item| item.max_stack);
            match entity.items.get(1).and_then(Option::as_ref) {
                None => result.count <= limit,
                Some(output) => {
                    output.item == result.item
                        && output.damage == result.damage
                        && output.extra == result.extra
                        && output.count + result.count <= limit
                }
            }
        });
        let progress = match entity.nbt.get(&"progress".into()) {
            Some(Tag::Int(n)) => *n,
            _ => 0,
        };
        let (progress, done) = super::machine_sample::advance(progress, result.is_some());
        entity.nbt.insert("progress".into(), Tag::Int(progress));
        entity.data = vec![progress];
        if done {
            let input = entity.items[0].as_mut().unwrap();
            input.count -= 1;
            if input.count == 0 {
                entity.items[0] = None;
            }
            let result = result.unwrap();
            if let Some(output) = &mut entity.items[1] {
                output.count += result.count;
            } else {
                entity.items[1] = Some(result);
            }
        }
        if game.native_block([pos[0] + 1, pos[1], pos[2]]) == super::machine_sample::PIPE {
            if let Some(output) = &mut entity.items[1] {
                let one = ItemStack {
                    count: 1,
                    ..output.clone()
                };
                if game
                    .native_insert_item([pos[0] + 2, pos[1], pos[2]], Direction::West, one)
                    .is_none()
                {
                    output.count -= 1;
                    if output.count == 0 {
                        entity.items[1] = None;
                    }
                }
            }
        }
    }
}
