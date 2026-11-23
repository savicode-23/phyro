@tool
extends EditorPlugin

const Importer := preload("atlas_importer.gd")
var importer: EditorImportPlugin
var pending := {}

func _enter_tree() -> void:
	importer = Importer.new()
	add_import_plugin(importer)
	EditorInterface.get_resource_filesystem().resources_reimported.connect(_pages_changed)

func _exit_tree() -> void:
	EditorInterface.get_resource_filesystem().resources_reimported.disconnect(_pages_changed)
	remove_import_plugin(importer)
	importer = null
	pending.clear()

func _pages_changed(paths: PackedStringArray) -> void:
	# Rotated crops are baked at import. Refresh their SpriteFrames even when
	# an artist re-exports only a PNG and leaves the layout file unchanged.
	var dirs := {}
	for path in paths:
		if path.get_extension().to_lower() == "png":
			var dir := path.get_base_dir()
			if not dirs.has(dir):
				dirs[dir] = []
			dirs[dir].append(path.get_file())
	for dir: String in dirs:
		for file in DirAccess.get_files_at(dir):
			if file.get_extension().to_lower() not in Importer.EXTENSIONS:
				continue
			var manifest := dir.path_join(file)
			if paths.has(manifest):
				continue
			var data: Variant = JSON.parse_string(FileAccess.get_file_as_string(manifest))
			if not data is Dictionary:
				continue
			var images: Variant = data.get("images", [data.get("image")])
			if images is Array and images.any(func(name): return name in dirs[dir]):
				pending[manifest] = true
	if not pending.is_empty():
		_refresh.call_deferred()

func _refresh() -> void:
	if pending.is_empty() or not is_inside_tree():
		return
	var files := PackedStringArray(pending.keys())
	pending.clear()
	EditorInterface.get_resource_filesystem().reimport_files(files)
