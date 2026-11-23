extends SceneTree

const ART := "res://art/lantern-patrol"
const CLIP := &"Lantern patrol"
var heard: Array = []
var character: AnimatedSprite2D

func _initialize() -> void:
	call_deferred("_verify")

func _record(event_name: StringName, detail: Dictionary) -> void:
	heard.append([String(event_name), int(detail.frame), character.frame])

func _verify() -> void:
	if not verify_animation_set():
		return
	var scene: PackedScene = load("res://packed.tscn")
	if scene == null:
		fail("The packed scene did not import.")
		return
	var demo := scene.instantiate()
	character = demo.get_node("Character")
	character.connect("animation_event", _record)
	root.add_child(demo)
	var frames := character.sprite_frames
	if frames == null or not frames.has_animation(CLIP) or frames.get_frame_count(CLIP) != 12 or frames.get_animation_speed(CLIP) != 12.0 or not frames.get_animation_loop(CLIP):
		fail("The imported SpriteFrames has the wrong clip timing.")
		return
	var cache := {}
	var unique: Array[PackedByteArray] = []
	var boots := PackedByteArray()
	var trimmed := false
	var rotated := false
	for i in frames.get_frame_count(CLIP):
		var entry := frames.get_frame_texture(CLIP, i) as AtlasTexture
		if entry == null or entry.atlas == null or Vector2i(entry.get_size()) != Vector2i(128, 128) or frames.get_frame_duration(CLIP, i) != 1.0:
			fail("Invalid atlas texture, restored size or duration at frame %d." % i)
			return
		var page := str(entry.get_meta(&"loopline_page"))
		if not cache.has(page):
			cache[page] = png(page)
		var atlas: Image = cache[page]
		var reference := png(ART + ("/reference/frame_%04d.png" % i))
		if atlas == null or reference == null:
			fail("A PNG is missing at frame %d." % i)
			return
		trimmed = trimmed or entry.region.size != entry.get_size()
		var restored := Image.create_empty(128, 128, false, Image.FORMAT_RGBA8)
		var raw_crop := atlas.get_region(Rect2i(entry.get_meta(&"loopline_region")))
		if entry.get_meta(&"loopline_rotated"):
			rotated = true
			raw_crop.rotate_90(COUNTERCLOCKWISE)
		restored.blit_rect(raw_crop, Rect2i(Vector2i.ZERO, raw_crop.get_size()), Vector2i(entry.margin.position))
		var bytes := restored.get_data()
		if bytes != reference.get_data():
			fail("Frame %d changed after packing, import and restoring its transparent border." % i)
			return
		var engine_crop := entry.get_image()
		if engine_crop == null or engine_crop.get_size() != Vector2i(entry.region.size):
			fail("Godot could not read the imported atlas texture at frame %d." % i)
			return
		engine_crop.convert(Image.FORMAT_RGBA8)
		var engine_frame := Image.create_empty(128, 128, false, Image.FORMAT_RGBA8)
		engine_frame.blit_rect(engine_crop, Rect2i(Vector2i.ZERO, engine_crop.get_size()), Vector2i(entry.margin.position))
		if not visible_pixels_match(engine_frame.get_data(), bytes):
			fail("The imported texture changed the visible pixels at frame %d." % i)
			return
		if not unique.has(bytes):
			unique.append(bytes)
		var feet := restored.get_region(Rect2i(28, 106, 54, 10)).get_data()
		if i == 0:
			if restored.get_pixel(38, 110).a < 0.9:
				fail("Expected an opaque boot near the origin.")
				return
			boots = feet
		elif boots != feet:
			fail("The boots shifted between packed frames.")
			return
	if not trimmed or not rotated or cache.size() < 2 or unique.size() < 8:
		fail("Expected multiple pages, rotated and trimmed frames, and eight distinct poses.")
		return

	# --fixed-fps 120 makes one tick of this 12 fps clip ten process frames.
	var previous := character.frame
	var transitions := 0
	var last_change := 0
	for step in range(1, 175):
		await process_frame
		if character.offset != Vector2(-56, -114) or character.position != Vector2(192, 236):
			fail("The world-space origin moved during playback.")
			return
		if character.frame == previous:
			continue
		if character.frame != (previous + 1) % 12 or (transitions > 0 and absi(step - last_change - 10) > 1):
			fail("Incorrect sequence or exposure duration at frame %d." % character.frame)
			return
		previous = character.frame
		last_change = step
		transitions += 1
		if transitions == 12:
			var expected := [["light_on", 0, 0], ["swing_left", 2, 2], ["bell", 2, 2], ["swing_right", 7, 7], ["light_on", 0, 0]]
			if heard != expected:
				fail("Unexpected events or tick order: %s" % str(heard))
				return
			print("PASS: 12 packed frames match native PNGs, origin and feet stay fixed, 12 fps playback loops with ordered events.")
			quit(0)
			return
	fail("Timed out before the animation looped.")

func verify_animation_set() -> bool:
	var actions := load(ART + "/actions.phyro") as SpriteFrames
	if actions == null or actions.get_animation_names() != PackedStringArray(["Idle", "Wave"]):
		fail("The animation set did not import as two named actions.")
		return false
	if actions.get_animation_speed(&"Idle") != 6.0 or actions.get_animation_speed(&"Wave") != 18.0 or not actions.get_animation_loop(&"Idle") or actions.get_animation_loop(&"Wave"):
		fail("The animation set lost individual action timing.")
		return false
	if actions.get_frame_count(&"Idle") != 2 or actions.get_frame_duration(&"Idle", 0) != 2.0 or actions.get_frame_duration(&"Idle", 1) != 2.0 or actions.get_frame_duration(&"Wave", 1) != 3.0:
		fail("An event on a held drawing did not split at the right local tick.")
		return false
	var events: Dictionary = actions.get_meta(&"loopline_events")
	var pivots: Dictionary = actions.get_meta(&"loopline_pivots")
	if events[&"Idle"][1][0].name != "glow" or events[&"Wave"][0][0].name != "lift" or pivots[&"Idle"][1] != Vector2(56, 114) or pivots[&"Wave"][1] != Vector2(20, 22):
		fail("Pivots or events were assigned to the wrong animation.")
		return false
	var a := actions.get_frame_texture(&"Idle", 0) as AtlasTexture
	var b := actions.get_frame_texture(&"Wave", 1) as AtlasTexture
	if a.atlas != b.atlas or a.get_size() != Vector2(128, 128):
		fail("The two actions did not share the restored rotated texture.")
		return false
	return true

func png(path: String) -> Image:
	var image := Image.new()
	if image.load_png_from_buffer(FileAccess.get_file_as_bytes(path)) != OK:
		return null
	image.convert(Image.FORMAT_RGBA8)
	return image

func visible_pixels_match(imported: PackedByteArray, original: PackedByteArray) -> bool:
	# Godot extends edge RGB during texture import. Compare exact alpha and
	# premultiplied color so invisible texels have no weight.
	var total := 0.0
	for i in range(0, original.size(), 4):
		var alpha := original[i + 3]
		if imported[i + 3] != alpha:
			return false
		for channel in 3:
			total += absi(imported[i + channel] - original[i + channel]) * (alpha / 255.0)
	return total / original.size() < 0.2

func fail(message: String) -> void:
	push_error(message)
	quit(1)
