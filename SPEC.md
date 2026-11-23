# Phyro
## Product and Engineering Specification for Codex

This specification records the original product goals and later design proposals. For current capabilities and priorities, see the [roadmap](ROADMAP.md). For implementation details, see the [architecture map](ARCHITECTURE.md). The [editor guide](docs/editor-guide.md) and [command-line guide](docs/command-line.md) describe the workflows available in the current source tree.

**Project:** Cross-platform Vector Graphics and 2D Animation Editor  
**Primary language:** Rust  
**Platforms:** Windows and Linux  
**Primary audience:** Indie game developers, game artists, animators and UI designers  
**Primary objective:** Create a professional, intuitive vector graphics and animation application with exceptionally efficient workflows for producing game-ready assets.

---

# 1. Product Vision

Build a native desktop application combining the vector editing capabilities of Inkscape with modern animation workflows inspired by Moho, Adobe Animate and Spline.

The application should make it possible to:

1. Create professional vector graphics from scratch.
2. Edit complex vector illustrations using familiar tools.
3. Animate any vector object or property.
4. Create frame-by-frame animations.
5. Build reusable character rigs and animate them.
6. Create reusable symbols and animated components.
7. Export individual animation frames, sprite sheets and animation atlases.
8. Integrate exported animations into game engines, particularly Godot.
9. Perform all these operations within one consistent editing environment.
10. Let an AI agent inspect a project, make precise edits and verify the exported result using the same document model as the desktop editor.

The guiding principle is:

**Drawing, animating and exporting game assets should feel like one continuous creative workflow.**

This must be a real editor, not a mockup or demonstration of graphical UI components.

The application should eventually be viable for serious production work.

## 1.1 Core design principles

**Direct manipulation:** Users should primarily interact with artwork on the canvas. Visual handles and intuitive interactions should be preferred over entering numerical values.

**Non-destructive editing:** Preserve original objects, vector paths, animation information and editable parameters whenever possible.

**Animation-first architecture:** Everything should be structured so that it can potentially be animated, even if animation features are not yet implemented.

**Fast iteration:** Editing an animation and exporting updated game assets should require very few interactions.

**Predictable behavior:** Identical project states and export settings must produce equivalent output.

**Performance:** Maintain fluid interaction with reasonably complex illustrations and animations.

**Professional precision:** Provide snapping, guides, numeric input, alignment, fine node editing and precise transforms alongside intuitive direct manipulation.

**Creative flexibility:** Support handcrafted visual styles rather than pushing artists toward predefined templates or procedural artwork.

## 1.2 Scope boundaries

This is primarily a 2D vector application.

Spline is a reference for interaction design and state-based animation, not a requirement to implement a 3D modeling engine.

Do not initially implement:

- Full 3D modeling.
- Cloud collaboration.
- Online accounts or subscriptions.
- An application marketplace.
- Generative AI image creation.
- A complete raster painting engine.
- Compatibility with proprietary project formats such as FLA.
- Every existing Inkscape extension.

These can be reconsidered after the application has a reliable editing and animation foundation.

---

# 2. Technology Requirements

## 2.1 Language and platform

Use Rust for the application, document model, rendering integration, editing tools, animation engine and export pipeline.

Target:

- Windows 10/11, x86-64.
- Modern desktop Linux distributions, initially x86-64.
- Linux X11 and Wayland.
- High-DPI displays.
- Standard mouse and keyboard.
- Graphics tablets as a later enhancement.

The application must work completely offline.

Use the stable Rust toolchain and Cargo workspaces.

## 2.2 Recommended technology stack

These are preferred technologies, not unchangeable requirements.

| Area | Preferred approach |
|---|---|
| Desktop UI | egui / eframe |
| Graphics API | wgpu |
| Vector rendering | Evaluate Vello CPU/GPU and alternatives |
| Vector geometry | kurbo |
| Path tessellation | lyon where appropriate |
| SVG import | SVG-aware parsing with usvg as a rendering/reference component |
| SVG rendering/export validation | resvg |
| Serialization | serde |
| Native asset packaging | ZIP-based project container |
| Image processing | image |
| File dialogs | rfd |
| Error handling | thiserror / anyhow |
| Logging | tracing |
| Testing | Rust unit, integration, snapshot and property-based tests |

**Important:** Before committing to a rendering stack, create a small technical prototype testing actual requirements, including antialiased paths, strokes, masks, gradients, large zoom levels and headless rendering.

Do not assume that a rendering library supports all necessary features. Introduce an abstraction so the rendering implementation can evolve without changing the document model.

Use a native graphical application. Do not wrap a browser application merely to obtain desktop support.

Avoid unnecessary architectural complexity and dependencies.

Pin compatible dependency versions, commit Cargo.lock and document important technology decisions.

---

# 3. User Interface and Experience

The software should feel modern and approachable without sacrificing professional editing capabilities.

## 3.1 Workspace layout

The default workspace should contain:

**Top toolbar**
- File operations.
- Undo and redo.
- Active tool options.
- Snapping options.
- Zoom controls.
- Preview and export actions.

**Left tool panel**
- Selection.
- Node editing.
- Pen.
- Pencil.
- Shapes.
- Text.
- Fill.
- Eraser.
- Other drawing tools.

**Central canvas**
- Infinite pasteboard surrounding finite artboards.
- Artists can arrange, select and transform objects across the pasteboard, including artwork wholly outside an artboard. The artboard provides a reference for alignment and an explicit export area.
- Smooth pan and zoom.
- Show transparent artboard areas on solid neutral gray by default. Offer **View → Transparency checkerboard** in the editor and use the same preference when editing symbol artwork. Both views preserve the alpha channel in exports.
- Pixel grid.
- Guides and rulers.
- Selection and transformation handles.

**Docked panels**
- Document structure and layers.
- Show the selected artwork thumbnail, name and layer above Appearance and Transform. Give fill and stroke direct hex, swatch and eyedropper controls. Show layers, groups and objects in an expandable tree with visibility and layer locks. Scroll the tree independently with its actions in reach.
- Fill and stroke.
- Transformations.
- Assets and symbols.
- Animation properties.

**Bottom panel**
- Animation timeline.
- Playback controls.
- Keyframes and exposure cells.
- Animation clips.
- Onion skin settings.
- Keyframe curve editor.

Allow Properties, Layers, Rig and Library to share tabs or occupy separate, resizable groups on either side of the canvas. Show Properties and Layers together in a new workspace.

Remember the dock positions, dimensions, active tabs, visible panels, timeline scale and artboard backdrop across launches as per-user workspace preferences.

Support saved workspace arrangements.

New projects start with a 1000 × 1000 pixel artboard. Dropping a native `.vanim` file anywhere in the window opens it through the same validation and unsaved-changes flow as **File → Open**. **File → Open Recent** lists the last ten successfully opened or saved native projects, most recent first. **File → Artboard size…** changes the shared project dimensions in pixels, keeping artwork in place across all clips and drawings. The artist can undo a resize in one step.

Provide a dedicated **Animation Workspace** that prioritizes the timeline and a **Drawing Workspace** that prioritizes vector editing.

## 3.2 Essential canvas interactions

Implement familiar editing interactions:

- Mouse wheel zoom with configurable behavior.
- Space + drag to pan.
- Click to select.
- Shift-click to modify selection.
- Drag to marquee-select.
- Double-click to enter groups or edit vector nodes.
- Escape to cancel active operations.
- Delete to remove selected objects.
- Ctrl+C / Ctrl+V for copy and paste.
- Ctrl+D duplicates the selected objects in place on their original layers. Select the new copies, retain their animation and rig bindings, and give copied groups independent identities. Each press is one undoable edit.
- Ctrl+Z / Ctrl+Shift+Z for undo and redo.
- Show Undo, Redo, Cut, Copy, Paste, Duplicate, Delete, Select All and Deselect together in the Edit menu with the same shortcuts and selection behavior as the keyboard commands.
- Shift constraints for drawing and transforming.
- Searchable keyboard shortcuts for tools, editing, path operations and animation. Show conflicts before applying a binding, support clearing and restoring defaults, and save changes in per-user preferences.

Support mouse-centered zoom, smooth camera movement and zoom-to-selection.

Selections must be stable when zooming, panning and editing.

All destructive commands must be undoable.

A drag interaction should generally create one undo operation rather than hundreds of separate history entries.

## 3.3 User experience requirements

Provide immediate feedback when drawing or transforming objects.

Tools should display contextual handles and relevant settings.

Avoid excessive dialogs and modal workflows.

Actions should be discoverable through contextual menus, tooltips and searchable commands.

Prioritize keeping the canvas uncluttered while still exposing powerful tools.

---

# 4. Vector Graphics Editor

The long-term goal is to cover the important everyday vector editing capabilities of Inkscape.

## 4.1 Basic drawing tools — V1

Implement:

- Rectangle and rounded rectangle.
- Ellipse and circle.
- Arc.
- Polygon.
- Star.
- Straight line.
- Polyline.
- Bézier pen.
- Selection and transformation.
- Node editing.

Shapes should remain editable as parametric shapes until explicitly converted to paths.

Keep the Bézier pen active after each completed path. The pen remembers its fill, stroke color and width during the editing session. Right-click or Enter finishes at the last authored node, leaving the stroke open. A fill can cover the area between the endpoints. Once the path has three nodes, snapping to the first node shows the closing curve before the artist clicks. Use a consistent screen-space snap distance across zoom levels.

Keep the finished path selected so changes to fill, stroke color and stroke width in Properties affect that path immediately. Starting the next path clears the previous selection. Changes made while drawing the new path affect its paint.

Use Inkscape’s drawing-tool shortcuts in the main editor and wherever those tools are available in symbol editing. Use S for Select, B for Bézier Pen, N for Nodes, R for Rectangle, E for Ellipse and T for Text. Use P or F6 for the freehand Pencil. Tool shortcuts should leave text and number entry intact. [Inkscape default shortcuts](https://gitlab.com/inkscape/inkscape/-/raw/master/share/keys/inkscape.xml)

Example: Rectangle corner radius should remain editable through a property rather than permanently changing the geometry.

## 4.2 Advanced drawing tools — V2

Add:

- Calligraphy brushes.
- Freehand Pencil for stylus and mouse with adjustable smoothing (Milestone 7).
- The Pencil can vary stroke width with tablet pressure. Preserve an editable centerline and a width profile along its length in the native project. The stroke width property sets the maximum width. Use a uniform stroke when pressure input is unavailable. Offer stronger freehand smoothing above the original 100% setting, up to 200%.
- Shape builder.
- Eraser with object and path-cutting modes.
- Curvature-oriented path drawing.
- Editable variable-width strokes.
- Brush profiles.
- B-spline or comparable spline editing.

## 4.3 Path editing — V1

Support:

- Add, remove and move nodes.
- Edit Bézier handles.
- With Nodes active, expose Corner (Shift+C), Smooth (Shift+S) and Symmetric (Shift+Y) in the top toolbar. Drag a box to select anchors and Shift-drag to add more. Apply the chosen style to the selected anchors in one undo step, respecting geometry keys. Corner retracts their handles on the first click. New corner handles move independently. Smooth handles stay collinear and Symmetric handles also keep equal length. Preserve the authored handle type when reopening the native project.
- Break paths.
- Join paths.
- Close and open paths.
- Reverse path direction.
- Subdivide segments.
- Simplify paths.
- Precise node positioning.
- Multi-node selection.
- Transform multiple nodes.
- Edit multiple paths.

Path editing must be reliable at high zoom levels.

In Nodes mode, double-click a path segment to insert and select a node at that point. Preserve the curve on both sides so the artist can drag the new node immediately. Double-clicking the edge of a plain rectangle or ellipse explicitly converts it to a path. One Undo restores the original shape.

The internal geometry model must preserve editable node identities whenever practical.

## 4.4 Boolean operations — V1

Implement:

- Union.
- Difference.
- Intersection.
- Exclusion.
- Division.
- Cut path.

Handle overlapping shapes, holes, self-intersections and fill rules consistently.

Keep curved Boolean results economical to edit. Preserve surviving Bézier spans and genuine corners where the resulting boundary can be traced to its operands. Check the anchor count and the filled silhouette after joining overlapping curved shapes, including a result reused as the next operand.

A straight cut through the top and bottom anchors of a four-node circle should leave three editable anchors on each half. Difference and Intersection keep the diameter straight. Cut Path returns open arcs with the same endpoints. Apply the same checks to filled open paths, concave outlines and holes. Where a boundary is ambiguous, preserve the geometry even if it needs more nodes.

Treat Boolean operations as a primary drawing workflow. An artist can Shift-select the next operand and apply a keyboard shortcut without opening a menu. Show the shortcuts in the Path menu. Keep the result selected and editable so the artist can move it, edit its nodes or use it as the next operand. Subtraction uses the stacking order of the selected shapes. Undo should restore the original geometry and stacking in one step. Include repeated operations on a small selection in a large document in responsiveness testing.

Provide clear error handling for unsupported or invalid geometry.

## 4.5 Non-destructive path effects — V2

Support a modifier stack capable of effects such as:

- Offset.
- Rounded corners.
- Simplify.
- Mirror and symmetry.
- Pattern along path.
- Repeater.
- Path distortion.
- Variable-width stroke.
- Envelope deformation.

The user should be able to edit the original shape and modify or remove the effects afterward.

Do not implement these effects through permanently destructive transformations unless explicitly requested.

## 4.6 Fill and stroke — V1

Support:

- Solid fills.
- RGBA colors.
- Linear gradients.
- Radial gradients.
- Rectangular gradients that fall off toward the edges of a box.
- Fill and stroke each have their own paint type, stops, and gradient geometry. The Gradient tool (**G**) lets the user choose the fill or stroke, draw a gradient across an object, and drag its handles to position and size the fade. In **Properties → Appearance**, show both paints and let the user choose which one’s stops to edit.
- Stroke width.
- Stroke alignment where supported.
- Line caps.
- Line joins.
- Dashes.
- Opacity.
- Fill rules.
- Press **D** for the eyedropper. Clicking samples the current drawing’s composited color and alpha into the selected objects’ fill. Shift-click samples into their stroke color. With no selection, update the drawing tool’s paint. The artboard background, onion skins and selection handles are display aids and do not contribute to the sampled color.
- **Edit → Paste style** (**Ctrl+Shift+V**) takes the evaluated fill and stroke colors, their gradients, and the stroke width from copied artwork and applies them to the selected editable shapes in one undoable edit. Geometry, stacking, opacity and unrelated animation tracks stay with the destination shapes.
- Remember fill, stroke, gradients, no-paint choices, stroke width, caps and joins between sessions. Keep a separate drawing style for the Pencil and offer the last eight colors in the picker.

V2:

- Conical or mesh gradients.
- Pattern fills.
- Advanced stroke profiles.
- Color palettes and swatches.
- Blend modes.

## 4.7 Object operations — V1

Implement:

- Move, rotate, scale and skew.
- Editable transform origin.
- Numeric transforms.
- Offer **Group** and **Ungroup** in the Object menu and the canvas context menu. Ctrl+G groups a same-layer selection. Ctrl+Shift+G releases its members for individual editing. Each command takes one undo step.
- Use the top selected vector path to clip an individual object or a group. **Set Clip** uses the path’s silhouette. **Set Mask** applies an adjustable alpha ramp through that silhouette. **Release Clip** and **Release Mask** restore the editable source path. Save its original geometry, style and stacking position with the project.
- Duplicate.
- Reorder.
- Press H to flip the selection left to right or V to flip it top to bottom. Use one shared center for a group or multiple selected objects. Each flip takes one Undo step. Keep both keys available for typing while editing text.
- Align.
- Distribute.
- Lock and hide.
- Organize into layers.
- Clip objects.
- Basic masks.

**Page Up** and **Page Down** move selected artwork past the next overlapping object or group within its layer. Compare bounding boxes at the current frame. **Home** brings the selection to the front of the layer and **End** sends it to the back. Groups move together. A multi-selection keeps its internal paint order, and one Undo restores the previous order.

Dragging a corner changes width and height independently around the opposite corner. Side handles change one dimension around the opposite edge. **Ctrl** uses the selection center as the pivot, **Shift** preserves proportions and **Ctrl+Shift** scales proportionally around the center. The modifiers work for individual objects, groups and multi-selections. Apply the same stretch to each member’s geometry and placement, including rotated members, while retaining the editable objects.

The Select tool shows a pivot crosshair on a single editable object. Drag it directly to choose the point that stays fixed during scaling and rotation, even outside the artwork. Moving the pivot preserves the pose at the current frame and takes one Undo step. Save the pivot with the object. **Reset pivot** restores the default transform behavior.

**Ctrl+Shift+A** opens **Align and Distribute** in the inspector and keeps the current selection. **Biggest selected** follows Inkscape’s rule. Left, Center X and Right keep the tallest item fixed. Top, Center Y and Bottom keep the widest item fixed. **Largest area** uses the same reference for both axes. Measure each object or group by its evaluated, axis-aligned bounds at the current frame, including stroke. If measurements tie, use the first item in document order. Keep a group's members in the same relative positions. For evenly spaced rows or columns, equalize the gaps between the selected items and keep the two ends fixed. Each command is one undoable edit.

V2:

- Advanced compositing.
- Nested clipping and masking.
- Live linked clones.
- Object-based repeaters.
- Advanced alignment and distribution.
- Multi-artboard documents.

## 4.8 Text — V1 and V2

V1:

- Select the Text tool with T or F8. A click on the artboard or pasteboard places the text cursor. A click on existing lettering edits that object in place.
- Enter adds a new line. Ctrl+Enter or a click outside finishes the edit. Escape discards the draft. Each finished label or revision is one Undo step.
- Font family and font size.
- While typing, draw the actual glyph outlines at the object's current transform. Show typeface, size, alignment and spacing controls within the workspace and preview their changes in place.
- Fill and stroke.
- Basic alignment and multiline text.
- Transform text objects.
- Convert text to paths.

V2:

- Kerning.
- Letter spacing.
- Line spacing.
- Text on paths.
- Text inside shapes.
- Variable fonts.
- Advanced Unicode shaping.
- Right-to-left layout.
- Font fallback.
- Rich text runs.

Use an established text shaping solution. Do not attempt to recreate a professional text layout engine from scratch.

## 4.9 Import and interoperability

Prioritize SVG import/export.

Preserve native editable objects when importing supported SVG elements.

Do not use a flattened rendering representation as the sole editable SVG import representation.

Support embedded and linked raster images.

Target formats:

- SVG — primary vector format.
- PNG — primary raster export format.
- JPEG/WebP — secondary bitmap formats.
- PDF — later.
- PSD layered import — later.
- Additional vector formats — later, according to demand.

If imported content uses unsupported SVG features, provide a warning and preserve or convert the content as faithfully as possible.

---

# 5. Animation System

Animation is a core architectural feature, not an optional plugin.

Objects must be addressable by stable identifiers so their properties can be animated independently.

Support two major animation styles:

**Frame-by-frame:** The user creates different drawings for individual frames.

**Property-based:** The user creates keyframes and the application interpolates editable properties.

Both approaches must work in the same document and animation clip.

## 5.1 Animation clips — V1

A document can contain multiple named clips, such as:

- idle
- run
- attack
- jump
- death

Each clip contains:

- Name.
- Frame rate.
- Duration in frames.
- Loop settings.
- Animation tracks.
- Keyframes.
- Frame exposures.
- Optional markers/events.
- Optional export settings overrides.

Clips should be reusable and independently previewable.

The document should have a clear base pose or base drawing separate from evaluated animation state.

Changing clips must not accidentally destroy the original document state.

## 5.2 Timeline — V1

Create a multi-layer timeline with:

- Frame ruler.
- Playhead.
- Add and delete individual keyframes. Box-select keys across tracks and drag or delete the selection in one Undo step. Preserve relative timing and easing, and keep existing keys when a destination is occupied.
- Copy and paste keyframes.
- Duplicate frames.
- Insert blank frames.
- Extend or shorten exposure.
- Drag frames to reorder or retime.
- Multi-frame selection.
- Playback.
- Pause.
- Loop playback.
- FPS controls.
- Adjustable timeline zoom.
- Named animation clips.

Differentiate visually between drawing exposures and property keyframes. Expose **Timeline scale** directly on the timeline. Use a compact 100% default independent of clip length and dock width and allow adjustment from 5% to 200%. At the smallest width, keep each frame selectable and space ruler labels far enough apart to read.

Expose frame rate beside the playhead counter for each clip. The default shortcuts are comma and period to step one frame, Space to play or pause, K to key the active property, F7 or Shift+K to start a blank drawing on the active layer, and Shift+D to duplicate the selected frames. Frame stepping may repeat when held. A single press creates a single key or drawing, and typing in a text field keeps keyboard focus.

Artists can draw effects over a moving reference. Inside a held drawing, a blank layer drawing begins at the playhead and preserves the timing of the shot. Other layers and placed clips, including placements on the active layer, keep their artwork and motion. A whole-frame insert remains available in the timeline's Clip menu.

The timeline should remain responsive for longer animations.

Selecting or deselecting artwork must preserve the canvas position and zoom. The timeline and inspector keep the dimensions chosen by the artist. Additional tracks and controls scroll within each panel, and the artist can drag either divider to resize it.

## 5.3 Frame-by-frame animation — V1

Support drawing different vector artwork on different frames.

Essential interactions:

- Create blank frame.
- Duplicate previous frame.
- Edit current frame.
- Copy or move a range of frames.
- Hold a drawing across several frames.
- Insert frames between existing frames.
- Replace an existing frame.
- Play the full animation.

Use a copy-on-write or similarly safe data model so duplicating a frame does not automatically cause later edits to modify every copy.

Users must be able to create traditional hand-drawn animations using vector tools.

## 5.4 Onion skinning — V1

Provide:

- Previous frame overlay.
- Next frame overlay.
- Adjustable past/future frame count.
- Configurable tint colors.
- Adjustable transparency.
- Independent toggles for previous and future frames.

The timeline offers a count of zero to eight distinct poses in each direction. Zero hides that side. Each direction has a color swatch with opacity. O toggles onion skins, Shift+O opens their settings, and Alt+comma or Alt+period cycles the previous or upcoming count.

V2:

- Onion skinning across selected ranges.
- Onion skinning by layer.
- Edit-multiple-frames mode.
- Advanced frame alignment/reference tools.

Onion skin overlays must never appear in exported animation frames.

## 5.5 Property keyframe animation — V1

Allow keyframes on:

- Position.
- Rotation.
- Scale.
- Skew.
- Opacity.
- Fill color.
- Stroke color.
- Stroke width.
- Visibility.

V2 adds:

- Bézier node positions.
- Bézier control handles.
- Gradient properties.
- Mask parameters.
- Effect parameters.
- Layer properties.
- Custom object parameters.
- Deformation controls.

When the playhead is positioned between keyframes, the application should interpolate animated properties without permanently modifying their base values.

Provide explicit auto-key mode and a manual keyframe mode.

## 5.6 Interpolation — V1

Support:

- Hold / stepped.
- Linear.
- Ease in.
- Ease out.
- Ease in-out.
- Cubic Bézier easing.

V2:

- Spring.
- Bounce.
- Elastic.
- Custom interpolation presets.
- Per-channel graph editor.

Geometry keys interpolate corresponding Bézier nodes on paths with matching layouts. Guided morphs also support a pair of single outlines with different node counts. Match two open paths or two closed paths automatically, and let the artist pair points anywhere along the curves to steer the in-between poses. Preserve the authored outlines at the keys and use the Geometry track’s easing.

Selecting two outlines and choosing Morph opens a playing preview in a movable, resizable window inside the editor. Keep the canvas and timeline visible around it. The first shape supplies the start pose and paint. The second supplies the end pose. Offer optional point matching on side-by-side outlines and a shared playhead for inspecting the result. **Done** commits the changes and any clip extension in one undoable edit. Closing the window discards the staged morph. Matching holes, splitting an outline and merging outlines are future work.

## 5.7 Animation curve editor — V2

Provide a graph editor for animatable numeric properties.

Support:

- Editable interpolation handles.
- Tangent control.
- Curve presets.
- Multiple tracks.
- Copy and paste curves.
- Independent property channels.
- Curve scaling.
- Time shifting.

Changes must update the scene preview immediately.

## 5.8 Motion paths — V2

Users should be able to define a motion path directly on the canvas.

An object can be attached to the path and animated along it.

Provide:

- Path position.
- Path direction.
- Align rotation to path.
- Adjustable timing.
- Editable control points.

## 5.9 Symbols and reusable objects — V1/V2

Create a reusable symbol library.

Dock the library alongside Properties, Layers and Rig. **View → Panels** controls which sidebar tabs appear. The Layers tree lists objects and supports renaming them with a double-click. Editing a symbol definition updates its linked placements throughout the project. Removing a definition keeps each placement as independently editable artwork, preserving its appearance, animation and position. One Undo restores the definition and its links.

Users can convert a group or illustration into a symbol and place multiple instances of it into the scene.

Symbol instances should support:

- Independent transforms.
- Reusable shared artwork.
- Independent animation timing.
- Nested symbol references.
- Instance overrides where practical.

Editing the underlying symbol should update its instances.

Guard against circular symbol references.

## 5.10 Nested animations — V2

Allow an animated symbol to be used inside another animation.

Example:

A character's body movement can run while a separate blinking animation plays on the face.

Support:

- Nested clips.
- Clip playback speed.
- Frame offset.
- Loop settings.
- Clip time remapping where appropriate.

Avoid making every animation a single enormous timeline.

---

# 6. Advanced Character Animation

These features are inspired by Moho and should be developed after the vector editor and basic animation system are mature.

## 6.1 Bone rigging — V2

Allow the user to:

- Create bones.
- Parent bones.
- Move bone pivots.
- Assign objects to bones.
- Create character skeletons.
- Animate bones using keyframes.
- Reparent bones.
- Change bone length.
- Lock bones.
- Hide rigging overlays.

Support forward kinematics first.

## 6.2 Inverse kinematics — V2

Implement:

- Two-bone IK.
- Multi-bone IK where practical.
- IK targets.
- Rotation limits.
- Joint constraints.
- FK/IK switching.
- Basic pole target controls.

These should be predictable and interactively editable.

## 6.3 Vector deformation — V2/V3

Support deforming vector artwork using bones.

Start with weighted transformations of vector nodes.

Then introduce:

- Automatic weights.
- Manual weight painting or editing.
- Mesh-based deformation.
- Free-form deformation lattices.
- Vertex pinning.
- Bend controls.

Preserve original editable geometry.

Do not destructively replace source paths when evaluating a rig.

## 6.4 Corrective poses — V3

Implement a mechanism inspired by smart-bone-driven corrective animation.

Example:

When an elbow bends to 90 degrees, the character's vector drawing can automatically transition into an artist-defined corrective pose.

Requirements:

- Bone/property-driven pose corrections.
- Editable corrective drawings.
- Interpolated correction influence.
- Reusable controls.
- Support for expressions and face direction changes.

These corrective poses should work with the existing animation and document model.

## 6.5 Drawing replacement / switch layers — V2/V3

Users should be able to create alternative drawings within an object.

Examples:

- Different mouth shapes.
- Different hand poses.
- Front and side views.
- Different facial expressions.

A discrete track selects the active drawing at a particular frame.

This should integrate with both traditional and rigged animation.

## 6.6 Physics-assisted motion — V3

Later features may include:

- Spring constraints.
- Bone dynamics.
- Follow-through.
- Secondary motion.
- Gravity.
- Wind.
- Particle effects.

The procedural results must be bakeable into deterministic animation frames for game-asset export.

---

# 7. State-Based Animation

Introduce a higher-level animation system inspired by the convenient state-based workflows found in modern interactive editors.

This is separate from the timeline.

## 7.1 States — V3

An animated object may have named states, such as:

- Default.
- Hovered.
- Pressed.
- Disabled.
- Open.
- Closed.

States can define different property values or reference existing animation clips.

## 7.2 Transitions — V3

Allow transitions between states with:

- Duration.
- Easing.
- Trigger.
- Optional delay.
- Interrupt behavior.

Examples:

- Default → Hovered.
- Hovered → Default.
- Closed → Open.

Use a visual state graph or similarly intuitive interface.

## 7.3 Game workflow

The editor should distinguish between:

1. Visual states that can be exported as separate frames.
2. Animation clips that can be baked to sprites.
3. Future runtime state machines that may be exported to supported game engines.

Do not make runtime state-machine export a prerequisite for sprite export.

---

# 8. Game Development Features

This area is a major product priority.

Game developers should not need several external applications to prepare artwork for use in a game.

## 8.1 Game asset document presets — V1

Offer presets for:

- 16×16.
- 32×32.
- 64×64.
- 128×128.
- 256×256.
- 512×512.
- Custom pixel dimensions.

Allow independent document coordinates and export resolution.

Provide transparent backgrounds by default for sprite projects.

The application should clearly distinguish vector scale from the pixel dimensions used during raster export.

## 8.2 Sprite sheet export — V1

This is a critical feature.

Users must be able to export a selected animation clip or multiple clips into a sprite sheet.

Support two packing modes.

**Uniform grid**

- Fixed frame width and height.
- Configurable rows and columns.
- Consistent frame alignment.
- Frame order.
- Optional padding.
- Optional margins.
- Transparent unused cells.

**Packed atlas**

- Efficient packing of nonuniform frames.
- Optional transparent-pixel trimming.
- Configurable padding.
- Edge extrusion to reduce texture bleeding.
- Maximum atlas dimensions.
- Multiple atlas pages if required.
- Optional duplicate frame elimination.
- Optional frame rotation, disabled by default.

Provide a live preview of the resulting sheet before export.

## 8.3 Sprite export configuration

Export options should include:

- Selected animation or all animations.
- Frame range.
- Output resolution.
- Scale multiplier.
- Background color or transparency.
- Antialiasing.
- Pixel snapping where relevant.
- Grid or atlas mode.
- Frame padding.
- Frame alignment.
- Trim transparency.
- Naming convention.
- Output folder.
- Export preset.
- Existing-file replacement policy.

Support transparent RGBA PNG and reliable alpha handling.

Avoid colored fringes around transparent sprite edges.

## 8.4 Individual frame export — V1

Users should be able to:

- Export the currently selected frame.
- Export a specified frame number.
- Export all frames as separate PNG images.
- Export selected frames.
- Export a particular layer or symbol.
- Batch-export multiple animation clips.

Example output:

```text
character/
  idle/
    idle_0000.png
    idle_0001.png
    idle_0002.png
  run/
    run_0000.png
    run_0001.png
    run_0002.png
```

Support fixed artboard bounds, selected-object bounds and trimmed bounds.

Ensure fixed-artboard frame exports remain aligned across the entire animation.

For illustration, support two selection workflows directly from the editor. **Export selection** writes the selected artwork together at its painted bounds. Batch export writes one SVG or PNG per selected top-level object or group. Selecting a member includes the whole group once. Each standalone SVG fits the artwork at the current frame and uses the object or group name from Layers as its filename, with safe substitutions and suffixes for collisions. Both workflows include selected artwork outside the artboard and preserve strokes, opacity, masks and group composition. Keep **Export artboard** available for output cropped to the page.

For animation, preserve a shared coordinate system across the chosen frame range. Exporting a moving selection as a sequence or sprite sheet can use the union of its painted bounds across all chosen frames. Keep the clip origin, timing and per-frame offsets when trimming so the game can reconstruct the authored motion, including empty exposures.

Use object IDs in the reference drawing. Across independent drawings, follow the matching object name on the matching named layer. The pair must identify a single object. Preserve missing poses as fully transparent frames and report ambiguous matches before publishing the output.

## 8.5 Animation metadata — V1

Sprite sheet exports should optionally generate JSON metadata.

Include:

- Format version.
- Atlas filenames.
- Animation names.
- Frame sequence.
- Atlas region for each frame.
- Original untrimmed dimensions.
- Trim offsets.
- Frame pivot/origin.
- Frame duration.
- Frame rate information.
- Loop settings.
- Animation events.
- Optional hitbox metadata.

Define coordinate systems explicitly.

Use stable top-left-origin atlas coordinates and document the relationship between sprite pivots, original artboard coordinates and trimmed regions.

Handle variable-duration frames correctly.

Do not silently lose origin alignment when trimming.

## 8.6 Pivot and origin controls — V1

Allow the user to set a sprite origin or pivot.

Examples:

- Center.
- Bottom center.
- Custom point.
- Feet position.

A shared pivot should apply across all frames by default.

Support per-frame overrides.

Display pivot guides on the animation canvas.

Changing sprite sheet packing must not change the actual intended origin of an animation.

## 8.7 Animation events — V2

Allow placing events at specific frames.

Example:

```text
Frame 4: footstep
Frame 8: footstep
Frame 12: attack_hit
```

Export these events as metadata.

Events should not require writing code inside the editor.

## 8.8 Hitboxes and collision shapes — V2

Allow drawing shapes associated with animation frames.

Support:

- Rectangular hitboxes.
- Polygon hitboxes.
- Hurtboxes.
- Interaction regions.
- Attachment points.
- Named sockets.

These shapes should be stored separately from visible artwork.

Do not rasterize them into exported images.

Export their geometry as metadata.

## 8.9 Godot integration — V1/V2

Godot 4 integration is a high-priority target.

V1:

- PNG sprite sheet export.
- Individual PNG frame export.
- Compatible animation naming.
- Consistent frame dimensions.
- Engine-neutral JSON metadata.
- A documented Godot import workflow.

V2:

Create a Godot integration package capable of importing exported atlas metadata and building an appropriate SpriteFrames resource for AnimatedSprite2D.

Where practical, also support creating an AnimatedSprite2D scene.

Preserve:

- Animation names.
- Playback speeds.
- Loop settings.
- Frame durations.
- Sprite origins.

V3 may add:

- Collision polygon import.
- Animation event integration.
- Skeletal rig export.
- Reusable runtime animation data.

Keep game engine exporters modular so Unity, Defold and other engines can be added later.

## 8.10 Export presets — V1

Allow users to create and save presets, such as:

- Godot sprite sheet.
- Individual PNG frames.
- Pixel art 64×64.
- High-resolution atlas.
- UI icon SVG.
- UI icon PNG.

Export presets belong to the document or a reusable user library.

## 8.11 One-click re-export — V1

After configuring an export once, the user should be able to click a single **Re-export** action.

The application should reuse previously saved settings.

This is essential for quick iteration between the drawing editor and a running game.

V2:

- Export multiple presets at once.
- Optional automatic export on save.
- Asset manifests.
- Headless command-line export.
- Engine-oriented import watching.

---

# 9. Document and Scene Architecture

The internal document model is one of the most important engineering decisions.

Do not model an animation as a collection of unrelated bitmap frames.

The document must preserve editable artwork and animation information.

## 9.1 Core document entities

Design typed Rust structures representing:

**Document**
- Unique document identifier.
- Document version.
- Canvas and artboards.
- Scene hierarchy.
- Asset library.
- Animation clips.
- Export presets.
- Document settings.

**SceneNode**
- Stable identifier.
- Parent reference.
- Node type.
- Local transform.
- Style properties.
- Visibility.
- Lock state.
- Children where applicable.
- Optional modifiers.

**Geometry**
- Editable shapes.
- Paths.
- Path nodes and handles.
- Raster references.
- Text.
- Shape parameters.

**AnimationClip**
- Name.
- Duration.
- Frame rate.
- Tracks.
- Exposures.
- Events.

**AnimationTrack**
- Target object identifier.
- Typed animatable property.
- Keyframe collection.
- Interpolation information.

**SymbolDefinition**
- Shared editable content.
- Local hierarchy.
- Optional independent animation.

**SymbolInstance**
- Reference to definition.
- Instance transform.
- Optional overrides.

**ExportPreset**
- Export mode.
- Target clips.
- Size and scaling.
- Packing settings.
- Metadata format.
- Destination.

Use stable IDs so renaming or reordering objects does not break animation tracks.

## 9.2 Separation of concerns

The system should clearly separate:

1. Persistent document data.
2. Current editor selection and tool state.
3. Animation evaluation.
4. Renderable scene generation.
5. Rendering.
6. Export.
7. Undo/redo history.

Animation playback must not permanently mutate the authored document.

The renderer should receive an evaluated view of the scene at a requested animation frame.

Editor overlays such as selections, guides, bones and onion skin controls must remain separate from renderable artwork.

## 9.3 Animation evaluation

Design a deterministic evaluation pipeline:

```text
Document
    ↓
Select animation clip
    ↓
Evaluate at frame/time
    ↓
Apply property animation
    ↓
Resolve symbols and hierarchy
    ↓
Apply rigs and deformations
    ↓
Apply geometry modifiers
    ↓
Produce renderable scene
    ↓
Viewport or export renderer
```

The exact evaluation order must be documented, particularly for transforms, constraints, symbols and deformations.

Use one conceptual scene evaluation path for viewport preview and final export.

## 9.4 Undo/redo architecture

Use a command-based or transaction-based history system.

Requirements:

- Undo individual editing operations.
- Group drag updates into transactions.
- Undo timeline operations.
- Undo property changes.
- Undo object hierarchy changes.
- Undo bone modifications.
- Redo correctly after undo.
- Avoid excessive copying for large documents.

History must operate on structured document changes rather than UI state snapshots whenever practical.

## 9.5 Project format

Define a native project format, provisionally named `.vanim`.

Use a structured archive containing:

```text
project.vanim
  manifest.json
  document.json
  assets/
  thumbnails/
```

Requirements:

- Schema version.
- Forward migration strategy.
- Embedded or linked asset support.
- Human-inspectable structured data where practical.
- Atomic save operations.
- Background autosave for recoverable drafts, including untitled projects. Queue local checkpoints two seconds after the last completed edit, with a fifteen-second deadline during continuous editing. After an interrupted session, offer the latest complete draft with its timestamp and open it as a copy with its own Save As destination.
- Missing asset warnings.
- Validation when opening.

Do not use SVG as the native project format because SVG alone does not represent the complete editing and animation model.

SVG remains an essential interoperability format.

---

# 10. Rendering Requirements

## 10.1 Canvas renderer

Implement:

- Antialiased vector geometry.
- Fill and stroke rendering.
- Transparency.
- Clipping.
- Gradients.
- Image rendering.
- Transform hierarchies.
- At high zoom, rasterize visible vector artwork and onion skins at display resolution once navigation settles. Keep panning responsive while sharper pixels arrive. Verify diagonal and curved edges at both 1× and 2× display scaling.
- Viewport clipping/culling.

V2:

- Blend modes.
- Advanced masks.
- Effects.
- Nested compositing.
- Render caches.

## 10.2 Export rendering

Export must operate independently of the UI.

The same file should render correctly in a headless environment.

Use deterministic rendering with well-defined color handling.

Provide a reliable CPU fallback where appropriate.

Test output for:

- Correct alpha.
- Stable clipping.
- Accurate bounds.
- Proper premultiplication.
- Correct compositing.
- Consistent scaling.
- Expected antialiasing.

## 10.3 Performance

Initial engineering targets:

- Responsive dragging and node manipulation.
- Smooth pan and zoom.
- Interactive playback of ordinary sprite animations.
- Stable behavior at high zoom.
- Low latency during tool interactions.
- No unnecessary full-document recomputation for small edits.

After establishing baseline performance, benchmark scenes with approximately 1,000 and 10,000 vector objects.

Prioritize multi-object transforms in the dense Lantern Walk benchmark. With 10,000 objects outside the artboard, a 1100 × 740 window, 2× display scale and 2× fit zoom, median steady-frame CPU time reached 69 ms for scaling two objects and 83 ms for rotating them. Concurrent builds were running. Repeat on an idle machine and profile the preview and texture update costs against the 16.7 ms frame budget.

Include a workload that keeps the same visible illustration while increasing the number of objects outside the artboard. Repeat with objects on the artboard but outside the zoomed viewport. Measure object picking, movement, scaling and rotation in both cases. Offscreen geometry that can reach the view through animation, transforms, strokes or nested placements must still appear correctly.

Aim for 60 FPS interactive responsiveness on representative mid-range desktop hardware for ordinary documents.

Measure actual performance rather than assuming GPU acceleration guarantees acceptable results.

Use:

- Scene invalidation.
- Cached geometry.
- Dirty-region tracking where beneficial.
- Spatial indexing for hit testing.
- Reuse of evaluated geometry.
- Background export tasks.
- Cancellation for expensive operations.

Never block the UI for long exports without feedback or cancellation.

---

# 11. Proposed Rust Workspace Structure

Use a Cargo workspace organized around meaningful, testable boundaries.

```text
vector-animation-studio/
│
├── Cargo.toml
├── Cargo.lock
├── README.md
├── ARCHITECTURE.md
├── ROADMAP.md
│
├── crates/
│   ├── editor-app/
│   │   └── Application entry point
│   │
│   ├── editor-ui/
│   │   └── Panels, workspaces, toolbars
│   │
│   ├── editor-tools/
│   │   └── Selection, pen, shapes, nodes
│   │
│   ├── document-core/
│   │   └── Scene graph and persistence
│   │
│   ├── vector-geometry/
│   │   └── Paths and geometry operations
│   │
│   ├── animation-core/
│   │   └── Clips, tracks and evaluation
│   │
│   ├── renderer/
│   │   └── Scene rendering
│   │
│   ├── asset-export/
│   │   └── PNG, atlases and metadata
│   │
│   └── file-io/
│       └── Import and serialization
│
├── integrations/
│   └── godot/
│
├── examples/
├── tests/
└── docs/
```

These are logical module boundaries; merge crates if the initial implementation would otherwise become unnecessarily fragmented.

Avoid circular dependencies.

In particular, document-core must not depend on editor-ui.

The export pipeline should not require running the editor's windowing system.

---

# 12. Implementation Roadmap

This project is too large to implement reliably in one attempt.

Build it through working milestones, prioritizing complete workflows over isolated feature counts.

Every milestone must result in software that compiles, launches and performs its stated functions.

## Milestone 0 — Technical foundation and vertical slice

**Goal: Prove the complete architecture with the smallest working example.**

Implement:

- Rust workspace.
- Native Windows/Linux app shell.
- Canvas.
- Basic rectangle creation.
- Object selection.
- Object movement.
- Basic document structure.
- Minimal animation clip.
- Two position keyframes.
- Play/pause preview.
- Save and reopen native project.
- Export one PNG.
- Export a short PNG frame sequence.

Create a basic end-to-end rendering and export pipeline.

Even if the editor only supports rectangles, it must actually be possible to draw, animate, save and export them.

**Acceptance test:**

Launch the application, draw a rectangle, animate it moving across five frames, save, reopen and export the frames successfully.

This milestone validates the most consequential architecture decisions before investing in a large feature set.

## Milestone 1 — Usable vector editor

Implement:

- Selection and multi-selection.
- Shapes.
- Pen tool.
- Basic node editing.
- Fill and stroke.
- Transform tools.
- Grouping.
- Layers.
- Undo/redo.
- Copy/paste.
- Basic SVG import/export.
- Guides and snapping.
- Basic alignment.

**Acceptance test:**

Create and edit a simple vector character illustration using multiple shapes, custom paths, groups and layers. Save it, reopen it and export an SVG and PNG.

The imported SVG should remain editable for the supported elements.

## Milestone 2 — Production-oriented sprite animation

Implement:

- Multiple named animation clips.
- Full basic timeline.
- Frame duplication.
- Frame-by-frame vector drawing.
- Exposure durations.
- Onion skinning.
- Playback.
- Position/rotation/scale/opacity animation.
- Basic interpolation.
- Sprite origins.
- Uniform-grid sprite sheet export.
- Individual frame exports.
- Export presets.
- One-click re-export.

**Acceptance test:**

Create a 128×128 character with eight hand-edited frames, preview the animation and export both individual PNGs and a four-column sprite sheet.

Import the exported frames or sheet into Godot and verify that the animation plays and remains aligned.

## Milestone 3 — Professional vector features

Implement:

- Advanced node editing.
- Reliable boolean operations.
- Clipping and masks.
- Improved typography.
- Gradients.
- Non-destructive path effects.
- More sophisticated snapping.
- Symbol library.
- Improved SVG compatibility.
- More advanced object management.

**Acceptance test:**

Import a moderately complex real-world SVG, edit paths and styles, use boolean operations, save the project and export the result with acceptable visual fidelity.

## Milestone 4 — Advanced timeline animation

Implement:

- Property track management.
- Animatable path geometry.
- Custom easing.
- Animation curve editor.
- Motion paths.
- Nested symbols.
- Nested animation clips.
- Animation events.
- Packed texture atlases.
- Godot importer/integration.

**Acceptance test:**

Animate a character containing multiple vector parts using a combination of property keyframes and frame-by-frame edits, export the animation and import it into Godot with its intended timing.

## Milestone 5 — Character rigging

Implement:

- Bone creation.
- Bone hierarchy.
- FK and IK.
- Bone animation.
- Basic skinning.
- Vector deformation.
- Weight controls.
- Drawing replacements.

**Acceptance test:**

Rig a simple character with arms and legs, animate a walking cycle using bones, correct the pose where necessary and export the animation as a sprite sheet.

## Milestone 6 — Advanced animation capabilities

Implement:

- Corrective bone-driven poses.
- Mesh deformation.
- Improved motion curves.
- Advanced constraints.
- State-based animation.
- Animation blending.
- Reusable motion templates.
- Physics-assisted secondary motion.

**Acceptance test:**

Create a reusable character with multiple named actions, controlled joints, corrective poses and animation transitions, then bake the required clips into game-ready assets.

## Milestone 7 — Production polish

Focus on:

- Performance optimization.
- Recover unsaved artwork for interrupted windows. Verify recovery after a forced close during a checkpoint, isolation between open windows and drawing and timeline responsiveness during background writes. Keep crash reports available for diagnosing the interruption.
- High-DPI support.
- Accessibility.
- Keyboard customization. In Settings, let artists search by action or shortcut, record a replacement, resolve conflicting bindings and restore defaults for one action or all actions. Persist changes across launches and show the active bindings in menus and tooltips. Cover drawing, Boolean and timeline commands, including punctuation keys on different keyboard layouts.
- Asset workflow improvements.
- Better file compatibility.
- Additional exporters.
- Packaging and installation.
- Documentation.
- Automated release builds.
- Extensive real-world testing.

**Freehand Pencil and tablet input.** Press P or F6 to draw with a stylus or mouse. Show the stroke while the pointer is down. Provide a smoothing control from 0% to 100% in the top toolbar, shared with the artwork editor and remembered between launches. On returning within 12 logical screen points of the start of a loop, show the same closing ring as the Pen. Releasing there joins the curve through a single editable seam anchor. Moving away continues the stroke. Short marks and straight retraces stay open. Releasing creates one editable Bézier path and one Undo step. Repeated strokes retain the chosen paint and smoothing setting.

Verify on a physical tablet by tracing the same short corners and long curves at two smoothing settings. Compare jitter, corner fidelity, node count and preview latency at Fit and while zoomed in. Edit a resulting node, undo one stroke, and save and reopen the drawing. Repeat the basic drawing test with a mouse.

## Milestone 8 — Agent-accessible editing and export

An AI agent working on behalf of an artist should be able to discover how to use Phyro, edit an existing project and return a result the artist can open and keep editing. Build on the desktop editor and exporters established in the preceding milestones.

- Ship a documented, noninteractive command-line interface on Windows and Linux. Include built-in help, examples and a machine-readable description of supported commands, arguments and results.
- Let an agent inspect artboards, layers, objects, clips, bones and keyframes by name and stable identifier. Report coordinate units, frame numbering and editable properties.
- Support an end-to-end workflow for creating or modifying vector artwork, setting animation keys, previewing a frame, saving a native project and exporting a clip. Use the editor’s validation, animation evaluator and renderers.
- Accept a batch of edits as one transaction. Return structured results with affected identifiers, output paths and actionable errors. A failed batch must leave the original project intact.

**Implementation order**

1. Ship read-only project inspection and single-frame PNG preview first. Built-in help and versioned JSON describe the available commands. An object lookup returns the clip ID, the start and duration of the held drawing, its layer and object IDs, and the project revision it came from. Ambiguous names return the available matches.
2. Add edits using shared document operations. Start with fill changes and position keys. Distinguish changing a drawing held across several ticks from keying one property at a specific tick. Validate a complete batch against the revision the agent inspected, then save an editable copy. Report the index of a failed edit and the affected drawing. Extend the same interface to Boolean operations, alignment, and editable text.
3. Add export through the existing clip and sprite-sheet exporters. Return the preview and output paths, frame dimensions, timing and pivot so the artist or agent can verify the result. Keep CLI processing outside the editor's interactive rendering loop.

**Acceptance test**

Give an agent a bundled character project and the instruction to recolor a named path, animate it across five frames and export a sprite sheet. Starting from built-in help, the agent must find the object, make the edits, inspect a rendered frame, save a new editable project and export the sheet without opening a window. Reopen that project in the desktop editor and compare its frames, timing and pivot with the agent’s export. Repeat with a missing object ID and verify that the diagnostic identifies the failed edit and the original file is unchanged.

### Live assistant drawing preview

When an assistant draws a lantern, let the artist see its outline while the assistant is still preparing the glass and flame. The unit of progress is a small, complete visual change, such as a path or a group of paths that form one feature. Each step arrives in a separate MCP call. A long path first appears when the assistant sends its geometry.

**Artist interaction**

- Open one live preview in the shared tab on the first valid step. Show the cumulative artwork, the step label, and the affected clip and frame. Follow incoming steps by default. Sample the preview independently of the working playhead, and offer the existing Original comparison.
- Selecting a completed step pins that version while more work arrives. **Follow latest** catches up. On completion, leave the preview available for review.
- **Keep through step N** closes the draft and commits exactly the steps through the displayed version as one undoable edit. If later steps exist, state how many will be discarded beside the button. **Discard draft** closes it and restores the view of the artist’s work. The artist can make either decision while the assistant is drawing. To continue after a decision, the assistant reads the current revision and starts a new draft.

**MCP contract**

| Tool | Behavior |
| --- | --- |
| `begin_draft(document, expected_revision, request_id)` | Pin the committed base in the paired tab. Return a session-scoped `proposal_id`, state `drawing` and step 0. An identical retry returns the same draft. |
| `append_draft_step(proposal_id, step, label, operations)` | Submit the next numbered step, starting at 1. Validate the full step against the preceding draft. Return its number and per-operation results, including scoped draft object IDs usable by the next step. A failed step leaves its number available for correction. |
| `finish_draft(proposal_id, last_step)` | Seal a nonempty draft for review if the step count matches. Return `ready`. Repeating the call returns its current status. |
| `get_proposal(proposal_id)` | Return the state and latest validated step. An artist decision reports `applied` with the kept step, committed revision and committed operation results, or `rejected`. Expiry and conflicting document edits report `expired` and `stale`. |

An identical append retry returns the original acknowledgement. Reusing a step number with different content or skipping a number returns a conflict with the expected next step. A closed draft answers late calls with its decision. Support existing single-batch clients through `propose_edits`. Ordinary inspection, save and export read the committed project.

**Editor integration**

Use the existing review slot, pairing checks, shared edit validator and editor history. Keep an ordered log of validated steps, the base and the latest copy-on-write draft. Reconstruct a selected earlier version on a worker. Limit each draft to 128 operations and 1 MiB of cumulative step payload. Renew the ten-minute review timeout on successful new steps and on finish. Switching tabs pauses access. Stopping sharing or disconnecting releases the draft. A local edit, undo or redo invalidates its base revision.

Render off the UI thread with one render in flight and at most one newer request queued. Identify each result by shared tab, revision, proposal, step, clip and tick. During a burst, show advancing versions and retain every validated step for review. Pin the version whose pixels are visible when the artist presses Keep. Check its identity and revision on the UI thread immediately before committing. Late worker results cannot reopen a closed draft.

**Acceptance**

1. Send the outline, glass and flame in separate calls. See the outline before sending the glass. Retry the glass step and verify one pane in the preview. Send an invalid flame step, correct it with the same step number and verify it appears once.
2. Pin the glass step while the flame arrives. Keep through glass with another append in flight. Verify the scoped IDs, paint order and held frames on both sides of the edit. Undo once to recover the original and redo once to recover the lantern through glass. Also exercise discard, local edits, tab switching and disconnect during rendering.
3. Use the 1,000-object assistant responsiveness scene. Target a 95th-percentile time under 250 ms from receipt of a modest step (one path with up to 32 nodes) to its first visible preview when steps arrive once per second. Measure burst catch-up, first drag and timeline stepping separately. Time spent generating the next tool call is outside this measurement.

---

# 13. Quality and Testing Requirements

Do not consider a feature complete because its UI exists.

Each implemented feature must function correctly and be covered by suitable tests.

## 13.1 Unit tests

Include tests for:

- Path operations.
- Transform composition.
- Geometry bounds.
- Scene hierarchy changes.
- Animation interpolation.
- Frame timing.
- Undo and redo.
- Project serialization.
- Packing calculations.
- Pivot and trim conversions.

## 13.2 Integration tests

Cover:

- Save/open round trips.
- SVG import/export.
- PNG rendering.
- Sprite sheet generation.
- Multi-clip export.
- Invalid document handling.
- Missing assets.
- Export preset persistence.

## 13.3 Visual regression tests

Use fixed example projects and reference renderings.

Compare rendering output within documented tolerances.

Test:

- Stroke appearance.
- Gradients.
- Clipping.
- Transparency.
- Frame alignment.
- Animation evaluation.
- Imported SVG geometry.

## 13.4 Platform validation

Maintain CI for Linux and Windows.

Run:

```text
cargo fmt --check
cargo clippy --workspace --all-targets
cargo test --workspace
cargo build --workspace
```

Add headless export integration tests.

Use actual Windows and Linux GUI smoke tests where CI infrastructure allows.

## 13.5 Error handling

The application must not crash during normal editing when encountering an unsupported SVG element, broken linked asset, malformed project or export failure.

Provide actionable errors.

After a crash, surface a local report on the next launch. Let the artist review it, add optional reproduction details, and copy it in one action. Keep recent reports accessible from Help. Capture the build revision, platform, available graphics information, recent editing actions and counts. Include a source location and stack when available. On an abrupt exit, identify the last known state and distinguish it from an observed failure. Project data, text content and personal file paths are outside the automatic report.

Handle unsuccessful saves and exports safely.

Do not silently overwrite user files unless the chosen workflow or preset explicitly authorizes it.

---

# 14. Development Rules for Codex

Follow these rules throughout implementation.

### Rule 1 — Real functionality first

Never substitute a fake or static UI for a requested working feature.

If something is not implemented, disable it or clearly mark it unavailable.

### Rule 2 — Make incremental progress

Implement one coherent subsystem or vertical slice at a time.

Do not attempt every feature in this specification simultaneously.

### Rule 3 — Preserve architectural boundaries

Do not mix rendering, document mutation, UI interaction and animation evaluation into one large class or module.

Prefer simple, explicit Rust types and maintainable code.

### Rule 4 — Avoid premature abstraction

Introduce abstractions when they solve actual architectural requirements.

Do not create elaborate plugin systems, dependency-injection infrastructure or generalized frameworks without a concrete need.

### Rule 5 — Protect authored data

All editing operations should preserve as much original user data as possible.

Animation evaluation must never overwrite base artwork unintentionally.

### Rule 6 — Verify code

After implementing a feature:

1. Format code.
2. Compile it.
3. Run appropriate tests.
4. Fix errors.
5. Verify the actual user workflow.
6. Update documentation.
7. Record any known limitations.

Do not report success based only on generated source code.

### Rule 7 — Manage scope

Keep ROADMAP.md focused on upcoming work. Put the next priority first and remove items when they ship. Document shipped features in release notes or user guides.

Distinguish full product requirements from the current milestone.

### Rule 8 — Avoid placeholder implementations

Temporary implementations are acceptable only if they allow a genuinely working workflow and are documented as temporary.

Do not create dozens of unused modules or empty interfaces for future features.

### Rule 9 — Explain important decisions

Maintain ARCHITECTURE.md with important choices and trade-offs.

Document:

- Rendering backend.
- Document format.
- Animation evaluation.
- Undo/redo model.
- SVG interoperability.
- Sprite exporter design.

### Rule 10 — Keep the app runnable

At every milestone, the main branch should produce a usable application.

Do not replace a working feature with an incomplete rewrite without good reason.

---

# 15. Definition of the First Useful Release

The first public alpha does not need Moho-level rigging or complete Inkscape feature parity.

It should already solve one valuable game development workflow extremely well:

**Draw a vector sprite, animate it frame by frame, preview the result and export game-ready assets.**

A user should be able to:

1. Create a sprite-sized document.
2. Draw with vector shapes and Bézier paths.
3. Organize artwork into layers.
4. Create multiple animation clips.
5. Duplicate and edit drawings across frames.
6. Use onion skinning.
7. Preview animations at the intended frame rate.
8. Animate basic properties using keyframes.
9. Export aligned transparent PNG frames.
10. Export a sprite sheet.
11. Re-export after making changes.
12. Save and reopen the project without losing editability.

If these workflows are genuinely good, the application already has a meaningful reason to exist.

Further Inkscape-level and Moho-level features should enhance that workflow rather than compromise it.

---

# 16. Immediate Instructions for Codex

Begin implementing this application.

**Start with Milestone 0 only.**

Before writing the full implementation:

1. Inspect the repository, if one already exists.
2. Select compatible Rust dependencies.
3. Investigate the vector renderer integration sufficiently to avoid building the core architecture on unsupported assumptions.
4. Briefly document the architecture.
5. Create the minimal functional workspace.

Then implement a complete vertical slice:

**Draw → Select → Animate → Preview → Save → Reopen → Export.**

Do not implement advanced rigging, full SVG editing, effects or extensive application settings in this first milestone.

The initial application must have a real canvas, working mouse interactions, visible objects, a minimal timeline and functional PNG export.

Use maintainable Rust code and write automated tests for the document and animation model.

At the end of the milestone:

- Ensure the project compiles.
- Run formatting, linting and tests.
- Explain how to launch the application.
- Summarize what works.
- Document what is incomplete.
- Update ROADMAP.md.
- Identify the next coherent milestone.

Do not claim implementation of functionality that has not been verified.

**The long-term goal is professional software, but each development step must produce something real, usable and maintainable.**
