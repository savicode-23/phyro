extends Node2D

func _ready() -> void:
	$Character.play_clip(&"Lantern patrol")

func _draw() -> void:
	draw_line(Vector2(36, 236), Vector2(348, 236), Color("#435156"), 1.0)
	draw_line(Vector2(192, 236), Vector2(192, 242), Color("#81968f"), 1.0)
