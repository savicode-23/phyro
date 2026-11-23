extends SceneTree

const ART := "res://art/lantern-walk"
const CLIP := &"Lantern walk"
const PLAYER := preload("res://addons/phyro/phyro_sprite.gd")

func _initialize() -> void:
	call_deferred("_verify")

func _verify() -> void:
	var library := load(ART + "/animation.phyro") as SpriteFrames
	if library == null or not library.has_animation(CLIP):
		fail("Generate and import the Lantern Walk atlas before running this check.")
		return
	if library.get_frame_count(CLIP) != 8 or library.get_animation_speed(CLIP) != 8.0 or not library.get_animation_loop(CLIP):
		fail("Expected an eight-frame looping walk at 8 fps.")
		return
	var atlas := read_png(ART + "/sheet.png")
	if atlas == null:
		fail("Missing exported walk atlas.")
		return
	var character := PLAYER.new()
	character.sprite_frames = library
	root.add_child(character)
	var contacts: Array[String] = []
	character.animation_event.connect(func(name: StringName, detail: Dictionary):
		contacts.append("%s@%d" % [name, detail.frame]))
	character.play_clip(CLIP)
	var transitions := 0
	var previous := -1
	for step in range(150):
		await process_frame
		if character.offset != Vector2(-56, -113):
			fail("The imported pivot moved at frame %d." % character.frame)
			return
		if character.frame == previous:
			continue
		var frame := character.frame
		if frame != transitions % 8:
			fail("Playback skipped a walk pose.")
			return
		if transitions == 8:
			if contacts != ["near_contact@0", "far_contact@4", "near_contact@0"]:
				fail("Unexpected foot contacts: %s" % [contacts])
				return
			print("PASS: eight walk poses match the exported frames, with the authored pivot and foot contacts through the loop.")
			quit(0)
			return
		var entry := library.get_frame_texture(CLIP, frame) as AtlasTexture
		var reference := read_png(ART + ("/frames/frame_%04d.png" % frame))
		if entry == null or entry.atlas == null or entry.atlas.resource_path != ART + "/sheet.png" or reference == null:
			fail("Missing texture or reference for frame %d." % frame)
			return
		if library.get_frame_duration(CLIP, frame) != 1.0 or atlas.get_region(Rect2i(entry.region)).get_data() != reference.get_data():
			fail("Imported atlas region or duration differs from native frame %d." % frame)
			return
		transitions += 1
		previous = frame
	fail("Timed out before a complete walk loop.")

func read_png(path: String) -> Image:
	var image := Image.new()
	if image.load_png_from_buffer(FileAccess.get_file_as_bytes(path)) != OK:
		return null
	image.convert(Image.FORMAT_RGBA8)
	return image

func fail(message: String) -> void:
	push_error(message)
	quit(1)
