# Godot project icon

`source.svg` is the stock 128 × 128 Godot project icon. It contains a rounded background, transformed groups, inherited fills, circles, quadratic and cubic segments, and a two-contour face. The Godot Engine logo is copyright © 2017 Andrea Calabró and is licensed under Creative Commons Attribution 4.0 International. [Upstream icon](https://github.com/godotengine/godot/blob/3911e0963d39b79a85cb329340c6918397d2d05e/editor/icons/DefaultProjectIcon.svg) · [Logo attribution](https://github.com/godotengine/godot/blob/master/misc/logo/LICENSE.txt) · [License](https://creativecommons.org/licenses/by/4.0/)

The edited examples demonstrate a recolored background, a moved nose anchor and a catchlight cut into the right pupil using **Difference**. The white eye underneath is visible through the hole. These changes are part of the Phyro import and export example.

Open `edited.vanim` in Phyro to inspect the editable result. To reproduce the edits and exports from the repository root, supply a new output directory.

```sh
cargo run --locked -p project-io --example edit_godot_icon -- icon-output
```

The generator imports the upstream SVG, edits the document, saves and reopens the native project, then exports SVG and PNG. The independent reference images in `crates/project-io/tests/fixtures` were rendered with Inkscape 1.4.2. The edited reference expresses the anchor change and pupil hole directly in SVG.

To rerun the comparison, execute `cargo test --locked -p project-io --test vector_acceptance -- --nocapture`. It measures mean absolute error over the four premultiplied RGBA channels, checks the catchlight at its expected location, and confirms that the native project retains the editable contours.
