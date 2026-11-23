use super::*;

#[test]
fn set_picker_creates_a_set_and_keeps_new_clips_and_undo_in_scope() {
    for width in [820., 1200.] {
        let mut w = EditorWindow::with_artboard(egui::vec2(width, 760.), [64, 64]);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        w.click("Sets");
        w.click("New set…");
        w.frame(vec![egui::Event::Text("Hero".into())]);
        w.click("Create set");
        let hero = w.editor.project.clips[w.editor.timeline.clip].set_id;
        assert_eq!(w.editor.project.animation_set(hero).unwrap().name, "Hero");
        let new_clip = w.timeline_icon("New clip");
        w.click_at(new_clip);
        w.frame(vec![]);
        w.frame(vec![egui::Event::Text("Run".into())]);
        w.click("Create clip");
        let clip = &w.editor.project.clips[w.editor.timeline.clip];
        assert_eq!(clip.name, "Run");
        assert_eq!(clip.set_id, hero);
        w.click("Hero");
        w.click("Animations");
        assert_eq!(w.editor.timeline.clip, 0);
        w.click("Animations");
        w.click("Hero");
        assert_eq!(w.editor.project.clips[w.editor.timeline.clip].set_id, hero);
        w.click("Sets");
        w.click("Remove set…");
        w.click("Move clips and remove");
        assert!(w.editor.project.animation_set(hero).is_none());
        assert_eq!(w.editor.project.clips.len(), 3);
        w.editor.restore(false);
        assert_eq!(
            w.editor
                .project
                .clips
                .iter()
                .filter(|clip| clip.set_id == hero)
                .count(),
            2
        );
    }
}

#[test]
fn clip_settings_rename_move_and_edit_the_active_clips_sprite_origin() {
    for size in [egui::vec2(800., 500.), egui::vec2(1200., 740.)] {
        let mut w = EditorWindow::with_artboard(size, [64, 40]);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let enemy = w.editor.project.add_animation_set("Enemy").unwrap();
        w.editor.capture_baseline();
        w.click("⋯");
        w.click("Rename clip…");
        w.frame(vec![]);
        w.frame(vec![egui::Event::Text("Walk".into())]);
        w.click("Rename");
        assert_eq!(w.editor.project.clips[0].name, "Walk");
        w.click("⋯");
        w.click("Move to set");
        w.click("Enemy");
        assert_eq!(w.editor.project.clips[0].set_id, enemy);
        w.click("⋯");
        w.click("Feet");
        assert_eq!(w.editor.project.clips[0].origin, [32., 40.]);
    }
}

#[test]
fn export_dialogs_preview_names_and_remember_which_actions_belong_in_the_set() {
    for size in [egui::vec2(800., 500.), egui::vec2(1100., 740.)] {
        let mut w = EditorWindow::with_artboard(size, [32, 32]);
        w.ctx.style_mut(|style| style.animation_time = 0.);
        let p = &mut w.editor.project;
        let hero = p.clips[0].set_id;
        p.rename_animation_set(hero, "Hero").unwrap();
        p.rename_clip(0, "Walk").unwrap();
        let boots = p.add_clip_in_set(hero, "Boot poses").unwrap();
        for n in 0..30 {
            p.add_clip_in_set(hero, &format!("Other {n}")).unwrap();
        }
        let enemy = p.add_animation_set("Enemy").unwrap();
        let enemy_walk = p.add_clip_in_set(enemy, "Walk").unwrap();
        w.editor.finish_edit();

        let export_set = w.timeline_icon("Export set");
        w.click_at(export_set);
        w.control("Export Hero");
        w.control("hero.png");
        w.click("PNG sequence per clip");
        w.control("hero_walk_1.png");
        w.click("Sprite sheet per clip");
        w.control("hero_walk.png");
        w.click("Boot poses");
        w.click("Close");
        assert!(!w.editor.exports.is_open());
        assert!(!w.editor.project.clips[boots].include_in_set_export);
        w.editor.restore(false);
        assert!(w.editor.project.clips[boots].include_in_set_export);
        w.editor.restore(true);
        assert!(!w.editor.project.clips[boots].include_in_set_export);

        w.click("Export");
        w.click("Export animation set…");
        w.click("None");
        w.control("Select at least one clip to export.");
        assert!(w.editor.project.clips[enemy_walk].include_in_set_export);
        w.click("All");
        w.control("hero_walk.png");
        w.click("Boot poses");
        w.click("Close");

        w.editor.timeline.clip = boots;
        for via_menu in [false, true] {
            if via_menu {
                w.click("Export");
                w.click("Export clip…");
            } else {
                let button = w.timeline_icon("Export clip");
                w.click_at(button);
            }
            w.control("Export Boot poses");
            w.click("PNG sequence per clip");
            w.control("hero_boot_poses_1.png");
            w.click("Shared sprite sheet");
            w.control("hero.png");
            w.click("Sprite sheet per clip");
            w.control("hero_boot_poses.png");
            w.control("Export to new folder…");
            w.click("Close");
            assert!(!w.editor.project.clips[boots].include_in_set_export);
        }
    }
}
