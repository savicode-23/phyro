extends SceneTree

const DEMO := preload("res://keeper.tscn")
const ART := "res://art/lantern-swing"
const CLIP := &"Lantern swing"

func _initialize() -> void:
	call_deferred("_verify")

func _verify() -> void:
	var demo := DEMO.instantiate()
	root.add_child(demo)
	var character: AnimatedSprite2D = demo.get_node("Character")
	var library := character.sprite_frames
	if library == null or not library.has_animation(CLIP):
		fail("The demo did not load the exported clip.")
		return
	if library.get_frame_count(CLIP) != 8 or not library.get_animation_loop(CLIP) or library.get_animation_speed(CLIP) != 8.0:
		fail("Expected a looping eight-frame animation at 8 fps.")
		return
	var sheet := read_png(ART + "/sheet.png")
	if sheet == null or sheet.get_size() != Vector2i(512, 256):
		fail("Expected a 4 × 2 sheet with 128 × 128 cells.")
		return
	var planted_feet := PackedByteArray()
	var drawings: Array[PackedByteArray] = []
	for i in 8:
		var frame := library.get_frame_texture(CLIP, i) as AtlasTexture
		var cell := Rect2i((i % 4) * 128, floori(i / 4.0) * 128, 128, 128)
		if frame == null or Rect2i(frame.region) != cell or library.get_frame_duration(CLIP, i) != 1.0:
			fail("Wrong atlas region or exposure at frame %d." % i)
			return
		if frame.atlas == null or frame.atlas.resource_path != ART + "/sheet.png" or Vector2i(frame.atlas.get_size()) != sheet.get_size():
			fail("Frame %d does not use the imported sheet texture." % i)
			return
		var png := read_png(ART + ("/frames/frame_%04d.png" % i))
		if png == null or png.get_size() != cell.size:
			fail("Could not load full-size frame %d." % i)
			return
		var pixels := png.get_data()
		if sheet.get_region(cell).get_data() != pixels or drawings.has(pixels):
			fail("Atlas cell %d does not match its distinct exported drawing." % i)
			return
		drawings.append(pixels)
		# Check actual grounded artwork across frames, independent of the JSON pivot.
		var feet := png.get_region(Rect2i(28, 106, 54, 10)).get_data()
		if i == 0:
			if png.get_pixel(38, 110).a < 0.9:
				fail("Grounded boot sample is transparent.")
				return
			planted_feet = feet
		elif feet != planted_feet:
			fail("The character's boots moved at frame %d." % i)
			return

	# Use --fixed-fps 120. Each exposure should last 15 process ticks at 8 fps.
	var previous := character.frame
	var last_change := 0
	var transitions := 0
	for tick in range(1, 180):
		await process_frame
		if character.offset != Vector2(-56, -114) or character.position != Vector2(192, 236):
			fail("The sprite's origin shifted during playback.")
			return
		if character.frame == previous:
			continue
		if character.frame != (previous + 1) % 8:
			fail("A frame was skipped or reversed during playback.")
			return
		if transitions > 0 and absi(tick - last_change - 15) > 1:
			fail("Unexpected frame duration: %d ticks." % (tick - last_change))
			return
		previous = character.frame
		last_change = tick
		transitions += 1
		if transitions == 8:
			print("PASS: 8 atlas regions match 8 PNGs, planted boots and origin remain fixed, 8 fps playback loops.")
			quit(0)
			return
	fail("Timed out waiting for one full animation loop.")

func read_png(path: String) -> Image:
	var png := Image.new()
	if png.load_png_from_buffer(FileAccess.get_file_as_bytes(path)) != OK:
		return null
	return png

func fail(message: String) -> void:
	push_error(message)
	quit(1)
