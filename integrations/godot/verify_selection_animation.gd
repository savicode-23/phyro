extends SceneTree

const PLAYER := preload("res://addons/phyro/phyro_sprite.gd")

func _initialize() -> void:
	call_deferred("_verify")

func pair(values: Array) -> Vector2i:
	return Vector2i(int(values[0]), int(values[1]))

func read_png(path: String) -> Image:
	var image := Image.new()
	if image.load_png_from_buffer(FileAccess.get_file_as_bytes(path)) != OK:
		return null
	image.convert(Image.FORMAT_RGBA8)
	return image

func expected_bounds(frames: Array) -> Rect2i:
	var bounds := Rect2i()
	for frame: Variant in frames:
		if frame != null:
			var rect := Rect2i(pair(frame.origin), pair(frame.size))
			bounds = bounds.merge(rect) if bounds.has_area() else rect
	return bounds if bounds.has_area() else Rect2i(0, 0, 1, 1)

func pixels(directory: String, library: SpriteFrames, clip: StringName,
		expected: Dictionary, metadata: Dictionary, bounds: Rect2i, pivot: Vector2) -> String:
	var sheet := read_png(directory + "/export/sheet.png")
	if sheet == null:
		return "Missing sheet PNG."
	if pair(metadata.origin) != bounds.position or pair(metadata.frame_size) != bounds.size:
		return "Common bounds differ from the union of independently exported poses."
	if metadata.source_start != expected.first or metadata.source_end != expected.last:
		return "The exported source range changed."
	if metadata.frames.size() != expected.frames.size() or library.get_frame_count(clip) != expected.frames.size():
		return "A held or empty frame was lost."
	if library.get_animation_speed(clip) != expected.fps or library.get_animation_loop(clip) != expected.looping:
		return "The imported speed or loop flag differs from the source clip."
	for i in expected.frames.size():
		var record: Dictionary = metadata.frames[i]
		var native: Variant = expected.frames[i]
		var target := Image.create_empty(bounds.size.x, bounds.size.y, false, Image.FORMAT_RGBA8)
		var offset := Vector2i.ZERO
		var size := Vector2i.ZERO
		if native != null:
			offset = pair(native.origin) - bounds.position
			size = pair(native.size)
			var asset := read_png(directory + "/reference/frame_%04d.png" % i)
			if asset == null or asset.get_size() != size:
				return "Invalid independent source PNG at frame %d." % i
			target.blit_rect(asset, Rect2i(Vector2i.ZERO, size), offset)
		if int(record.index) != i or int(record.source_frame) != int(expected.first) + i:
			return "Frame %d maps to the wrong source tick." % i
		if pair(record.trim_offset) != offset or pair(record.trim_size) != size:
			return "Frame %d has incorrect trim or blank-frame offsets." % i
		if Vector2(record.pivot[0], record.pivot[1]) != pivot or library.get_frame_duration(clip, i) != 1.0:
			return "Frame %d changed the pivot or duration." % i
		var texture := library.get_frame_texture(clip, i) as AtlasTexture
		if texture == null or texture.atlas.resource_path != directory + "/export/sheet.png":
			return "Frame %d is missing the imported sheet texture." % i
		var single := read_png(directory + "/export/frames/frame_%04d.png" % i)
		if single == null or single.get_data() != target.get_data():
			return "Frame %d PNG differs from the native pose in its artboard position." % i
		var region := Rect2i(texture.region)
		if region.size != bounds.size or sheet.get_region(region).get_data() != target.get_data():
			return "Imported atlas region %d differs from the native pose." % i
	return ""

func playback(library: SpriteFrames, clip: StringName, expected: Dictionary, pivot: Vector2) -> String:
	var player := PLAYER.new()
	player.sprite_frames = library
	root.add_child(player)
	var events: Array[String] = []
	player.animation_event.connect(func(event: StringName, detail: Dictionary):
		events.append("%s@%d/%d" % [event, int(detail.frame), int(detail.source_frame)]))
	var wanted: Array[String] = []
	for event: Dictionary in expected.events:
		wanted.append("%s@%d/%d" % [event.name, int(event.frame), int(event.source_frame)])
	for event: Dictionary in expected.events:
		if int(event.frame) == 0:
			wanted.append("%s@0/%d" % [event.name, int(event.source_frame)])
	player.play_clip(clip)
	var previous := -1
	var transitions := 0
	var count: int = expected.frames.size()
	for step in range(ceili(120.0 / expected.fps) * (count + 2)):
		await process_frame
		if player.offset != -pivot:
			return "The imported sprite moved its world pivot during playback."
		if player.frame == previous:
			continue
		if player.frame != transitions % count:
			return "Playback skipped a held or transparent pose."
		if transitions == count:
			return "" if events == wanted else "Expected events %s, received %s." % [wanted, events]
		transitions += 1
		previous = player.frame
	return "Playback timed out before a complete loop."

func _verify() -> void:
	var args := OS.get_cmdline_user_args()
	if args.size() != 1:
		push_error("Run check_selection_animation.py to generate the acceptance project.")
		quit(1)
		return
	var directory := "res://" + args[0]
	var expected: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(directory + "/expected.json"))
	var metadata: Dictionary = JSON.parse_string(FileAccess.get_file_as_string(directory + "/export/sheet.json"))
	var manifest := "animation.loopline" if args[0] == "lantern-walk" else "animation.phyro"
	var library := load(directory.path_join("export").path_join(manifest)) as SpriteFrames
	var clip := StringName(expected.name)
	if library == null or not library.has_animation(clip):
		push_error("Godot could not import the selection sheet as SpriteFrames.")
		quit(1)
		return
	var bounds := expected_bounds(expected.frames)
	var pivot := Vector2(expected.pivot[0], expected.pivot[1]) - Vector2(bounds.position)
	var error := pixels(directory, library, clip, expected, metadata, bounds, pivot)
	if error.is_empty():
		error = await playback(library, clip, expected, pivot)
	if not error.is_empty():
		push_error(error)
		quit(1)
		return
	print("PASS %s %d frames at %d fps, artboard bounds %s, sprite pivot %s" % [
		args[0], expected.frames.size(), expected.fps, bounds, pivot])
	quit(0)
