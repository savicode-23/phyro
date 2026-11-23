extends Node2D

const METADATA := "res://art/lantern-swing/sheet.json"

@onready var character: AnimatedSprite2D = $Character
var pivots: Array[Vector2] = []

func _ready() -> void:
	var data: Variant = JSON.parse_string(FileAccess.get_file_as_string(METADATA))
	if not data is Dictionary or data.get("format_version") != 1:
		push_error("Expected Phyro sprite sheet metadata version 1.")
		return
	var image_path := METADATA.get_base_dir().path_join(str(data.image))
	var sheet := load(image_path) as Texture2D
	if sheet == null:
		push_error("Could not load the imported sprite sheet: " + image_path)
		return

	var library := SpriteFrames.new()
	library.remove_animation(&"default")
	var clip := StringName(data.name)
	library.add_animation(clip)
	library.set_animation_speed(clip, float(data.fps))
	library.set_animation_loop(clip, bool(data.looping))
	for frame: Dictionary in data.frames:
		var atlas := AtlasTexture.new()
		atlas.atlas = sheet
		atlas.region = Rect2(frame.x, frame.y, frame.w, frame.h)
		atlas.filter_clip = true
		library.add_frame(clip, atlas, float(frame.duration_ticks))
		pivots.append(Vector2(frame.pivot[0], frame.pivot[1]))

	character.sprite_frames = library
	character.animation = clip
	character.frame_changed.connect(_place_pivot)
	_place_pivot()
	character.play()

func _place_pivot() -> void:
	# With centered=false, the local sprite offset starts at the region's top-left.
	character.offset = -pivots[character.frame]

func _draw() -> void:
	draw_line(Vector2(36, 236), Vector2(348, 236), Color("#435156"), 1.0)
	draw_line(Vector2(192, 236), Vector2(192, 242), Color("#81968f"), 1.0)
