extends AnimatedSprite2D

signal animation_event(event_name: StringName, detail: Dictionary)

var restarting := false

func _ready() -> void:
	centered = false
	sprite_frames_changed.connect(_place_pivot)
	animation_changed.connect(_place_pivot)
	frame_changed.connect(_on_frame_changed)
	_place_pivot()

## Start a Phyro clip at tick zero, including any event on its first tick.
func play_clip(clip: StringName) -> void:
	if sprite_frames == null or not sprite_frames.has_animation(clip):
		push_error("Unknown Phyro animation %s." % clip)
		return
	restarting = true
	play(clip)
	set_frame_and_progress(0, 0.0)
	restarting = false
	_on_frame_changed()

func _place_pivot() -> void:
	if sprite_frames == null:
		return
	var clips: Dictionary = sprite_frames.get_meta(&"loopline_pivots", {})
	var pivots: Array = clips.get(animation, [])
	if frame >= 0 and frame < pivots.size():
		offset = -Vector2(pivots[frame])

func _on_frame_changed() -> void:
	if restarting:
		return
	_place_pivot()
	if sprite_frames == null or not is_playing():
		return
	var clips: Dictionary = sprite_frames.get_meta(&"loopline_events", {})
	var frames: Dictionary = clips.get(animation, {})
	for event: Dictionary in frames.get(frame, []):
		animation_event.emit(StringName(event.name), event)
