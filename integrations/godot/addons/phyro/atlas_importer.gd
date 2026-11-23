@tool
extends EditorImportPlugin

const EXTENSIONS := ["phyro", "loopline"]

func _get_importer_name() -> String:
	# Godot stores this identifier in existing .import files.
	return "loopline.sprite_frames"

func _get_visible_name() -> String:
	return "Phyro SpriteFrames"

func _get_recognized_extensions() -> PackedStringArray:
	return PackedStringArray(EXTENSIONS)

func _get_save_extension() -> String:
	return "res"

func _get_resource_type() -> String:
	return "SpriteFrames"

func _get_import_order() -> int:
	# The PNG importer runs first so cold imports can link external textures.
	return 1

func _get_priority() -> float:
	return 1.0

func _get_preset_count() -> int:
	return 1

func _get_preset_name(_preset_index: int) -> String:
	return "Atlas"

func _get_import_options(_path: String, _preset_index: int) -> Array[Dictionary]:
	return []

func _get_option_visibility(_path: String, _option_name: StringName, _options: Dictionary) -> bool:
	return true

func _import(source_file: String, save_path: String, _options: Dictionary,
		_platform_variants: Array[String], _gen_files: Array[String]) -> Error:
	var json := JSON.new()
	if json.parse(FileAccess.get_file_as_string(source_file)) != OK or not json.data is Dictionary:
		push_error("Invalid Phyro JSON in %s at line %d." % [source_file, json.get_error_line()])
		return ERR_PARSE_ERROR
	var library := build(json.data, source_file.get_base_dir())
	if library == null:
		push_error("Invalid Phyro atlas description in %s." % source_file)
		return ERR_INVALID_DATA
	return ResourceSaver.save(library, save_path + "." + _get_save_extension())

func integer(value: Variant, minimum: int) -> bool:
	return (value is float or value is int) and is_finite(float(value)) and value == floor(value) and value >= minimum and value <= 2147483647

func number(value: Variant) -> bool:
	return (value is float or value is int) and is_finite(float(value))

func build(data: Dictionary, directory: String) -> SpriteFrames:
	if not integer(data.get("format_version"), 1) or int(data.format_version) > 3 or not data.get("name") is String or str(data.name).is_empty():
		return null
	var is_set := int(data.format_version) == 3
	var clips: Variant = data.get("animations") if is_set else [data]
	if not clips is Array or clips.is_empty():
		return null
	var images: Variant = data.get("images") if is_set else data.get("images", [data.get("image")])
	if not images is Array or images.is_empty():
		return null
	var pages: Array[Texture2D] = []
	for filename: Variant in images:
		if not filename is String or str(filename).get_file() != filename or str(filename).get_extension().to_lower() != "png":
			return null
		var path := directory.path_join(str(filename))
		var texture := ResourceLoader.load(path, "Texture2D") as Texture2D
		if texture == null:
			push_error("Missing atlas page %s." % path)
			return null
		pages.append(texture)
	var library := SpriteFrames.new()
	library.remove_animation(&"default")
	var pivots_by_clip := {}
	var events_by_clip := {}
	var rotated_crops := {}
	for clip: Variant in clips:
		if not clip is Dictionary or not append_animation(library, clip, pages, rotated_crops, pivots_by_clip, events_by_clip):
			return null
	# These serialized keys are also read by existing player scripts and scenes.
	library.set_meta(&"loopline_pivots", pivots_by_clip)
	library.set_meta(&"loopline_events", events_by_clip)
	return library

func append_animation(library: SpriteFrames, data: Dictionary, pages: Array[Texture2D],
		rotated_crops: Dictionary, pivots_by_clip: Dictionary, events_by_clip: Dictionary) -> bool:
	if not data.get("name") is String or str(data.name).is_empty() or library.has_animation(StringName(data.name)):
		return false
	if not integer(data.get("fps"), 1) or not data.get("looping") is bool or not data.get("frames") is Array or data.frames.is_empty():
		return false
	var events: Variant = data.get("events", [])
	if not events is Array:
		return false
	var events_at := {}
	for event: Variant in events:
		if not event is Dictionary or not integer(event.get("frame"), 0) or not event.get("name") is String or str(event.name).is_empty():
			return false
		var tick := int(event.frame)
		if not events_at.has(tick):
			events_at[tick] = []
		events_at[tick].append(event)
	var event_ticks := events_at.keys()
	event_ticks.sort()
	var next_event := 0
	var tick := 0
	var clip := StringName(data.name)
	library.add_animation(clip)
	library.set_animation_speed(clip, float(data.fps))
	library.set_animation_loop(clip, bool(data.looping))
	var pivots: Array[Vector2] = []
	var events_by_frame := {}
	for index in data.frames.size():
		var frame: Variant = data.frames[index]
		if not frame is Dictionary or not integer(frame.get("index"), 0) or int(frame.index) != index:
			return false
		var texture := frame_texture(frame, pages, rotated_crops)
		var pivot: Variant = frame.get("pivot")
		if texture == null or not pivot is Array or pivot.size() != 2 or not number(pivot[0]) or not number(pivot[1]) or not integer(frame.get("duration_ticks"), 1):
			return false
		var point := Vector2(pivot[0], pivot[1])
		var end_tick := tick + int(frame.duration_ticks)
		# Split a long exposure only where an event needs its own frame signal.
		while tick < end_tick:
			if next_event < event_ticks.size() and event_ticks[next_event] == tick:
				events_by_frame[pivots.size()] = events_at[tick]
				next_event += 1
			var stop := end_tick
			if next_event < event_ticks.size():
				stop = mini(stop, int(event_ticks[next_event]))
			library.add_frame(clip, texture, float(stop - tick))
			pivots.append(point)
			tick = stop
	if next_event != event_ticks.size():
		return false
	pivots_by_clip[clip] = pivots
	events_by_clip[clip] = events_by_frame
	return true

func frame_texture(frame: Dictionary, pages: Array[Texture2D], rotated_crops: Dictionary) -> AtlasTexture:
	for key in ["x", "y", "w", "h"]:
		if not integer(frame.get(key), 1 if key in ["w", "h"] else 0):
			return null
	for key in ["trim_x", "trim_y", "page"]:
		if not integer(frame.get(key, 0), 0):
			return null
	var rotated: Variant = frame.get("rotated", false)
	if not rotated is bool:
		return null
	var packed := Vector2i(frame.w, frame.h)
	var crop := Vector2i(packed.y, packed.x) if rotated else packed
	if not integer(frame.get("source_w", crop.x), 1) or not integer(frame.get("source_h", crop.y), 1):
		return null
	var source := Vector2i(frame.get("source_w", crop.x), frame.get("source_h", crop.y))
	var trim := Vector2i(frame.get("trim_x", 0), frame.get("trim_y", 0))
	var page := int(frame.get("page", 0))
	if page >= pages.size() or int(frame.x) + packed.x > pages[page].get_width() or int(frame.y) + packed.y > pages[page].get_height():
		return null
	if int(trim.x) + crop.x > source.x or int(trim.y) + crop.y > source.y:
		return null
	var rect := Rect2i(int(frame.x), int(frame.y), packed.x, packed.y)
	var texture := AtlasTexture.new()
	texture.set_meta(&"loopline_page", pages[page].resource_path)
	texture.set_meta(&"loopline_region", rect)
	texture.set_meta(&"loopline_rotated", rotated)
	if rotated:
		# AtlasTexture cannot rotate its UVs. Bake each distinct rotated crop
		# once at import and save a lossless portable texture with the resource.
		var key := "%d:%s" % [page, rect]
		if not rotated_crops.has(key):
			var page_image := pages[page].get_image()
			if page_image == null:
				return null
			var pixels := page_image.get_region(rect)
			pixels.rotate_90(COUNTERCLOCKWISE)
			var native := PortableCompressedTexture2D.new()
			native.keep_compressed_buffer = true
			native.create_from_image(pixels, PortableCompressedTexture2D.COMPRESSION_MODE_LOSSLESS)
			rotated_crops[key] = native
		texture.atlas = rotated_crops[key]
		texture.region = Rect2(Vector2.ZERO, Vector2(crop))
	else:
		texture.atlas = pages[page]
		texture.region = Rect2(rect)
	texture.margin = Rect2(Vector2(trim), Vector2(source - crop))
	texture.filter_clip = true
	return texture
