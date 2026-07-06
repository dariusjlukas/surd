# Plotting

A plot is a **symbolic value** in the engine — like
[`diff`](calculus.md#diff-d), the plot variable is taken by name and kept
symbolic while the curve expressions evaluate. The web app's frontend samples
and draws it; in the terminal REPL it simply prints as itself.

Sampling is the one deliberate exception to arbitrary precision: pixels are
already approximate, so curves are sampled at f64 — but *results* never are.

## `plot`

```
plot(f1, ..., fk, x, a, b)
```

One or more curves in the variable `x` over the window `[a, b]`. The last
three arguments are always the variable and the window; everything before
them is a curve.

```text
>> plot(sin(x), x, 0, 6)
plot(sin(x), x, 0, 6)
>> plot(sin(x), cos(x), x, 0, 2*pi)        # two curves, shared window
plot(sin(x), cos(x), x, 0, 2*π)
```

In the web app:

- Curves are drawn with **gaps at poles** — an asymptote is never bridged
  with a lying vertical line.
- Sampling is **adaptive**: 601 points for smooth curves, refining up to
  4,801 while the resolution fails a convergence test (each sample is
  checked against linear interpolation of its 2×-coarser neighbors).
  Oscillatory curves like `sin(50*x)` get the resolution they need instead
  of aliasing; a window even the cap can't resolve is labeled
  **⚠ undersampled** rather than silently drawn wrong.
- Plots **resample on pan/zoom**: the plot value carries the re-parseable
  text of its expression (workspace bindings already substituted), so any
  window is re-sampled through the same adaptive policy — zooming reveals
  detail instead of stretching stale samples.
- Drag pans; wheel zooms x; shift+wheel zooms y. Touching the y-axis
  switches it from auto-fit to manual until reset.

Because the variable is taken by name, a workspace binding doesn't collapse
the curve: after `x := 3`, `plot(x^2, x, 0, 1)` still plots the parabola,
not the constant 9.

## Titles and axis labels

```
plot(..., title = "...", xlabel = "...", ylabel = "...")
plot3d(..., title = "...")
```

Every plot form — curves, signals, scatter — takes optional trailing
keyword-string arguments. Labels are **mathtext**: plain text, with `$...$`
segments rendered as LaTeX math (`\$` is a literal dollar):

```text
>> plot(sin(w)/w, w, -10, 10, title = "response of $H(\omega)$", xlabel = "$\omega$ (rad/s)", ylabel = "gain")
```

- Backslashes in strings stay literal, so LaTeX needs no doubling —
  `"$\omega$"` just works. Only `\"` (a quote) and `\\` (a backslash) escape.
- Labels must come after all positional arguments, must be string literals,
  and each may appear once; anything else is an error, never a silent drop.
- `plot3d` takes only `title` for now — the surface view has no honest place
  to draw per-axis labels yet, so it refuses them rather than dropping them.
- Labels show in the live view and are baked into the **png** button's export
  and PDF reports (along with tick numbers and, for multi-curve plots, the
  legend).

## `scatter`

```
scatter(x, y)
```

Data points from two equal-length vectors, drawn as **markers** rather than a
connected line — for comparing measured data against a model. `scatter` is a
data value, not a plot on its own; `plot` draws it, alone or overlaid with
curves:

```text
>> plot(scatter(xs, ys))                    # just the points, window auto-fit
>> plot(scatter(xs, ys), 2*x + 1, x, 0, 10) # points + a line on shared axes
```

The natural use is checking a fit against its data. A fit returns its curve as
a `predict` function (see [stats](stats.md#fitted-models)), so it drops straight
into the plot — no need to spell the formula back out:

```text
>> m := stats.linfit(xs, ys)                # exact least-squares line
>> plot(scatter(xs, ys), m.predict, x, 0, 10)
```

`m.predict` is an ordinary function, so it plots like any curve — `m.predict`
on its own, or `m.predict(x)` applied to the variable. It also predicts at a
point: `m.predict(2.5)`.

In the web app:

- A scatter series is **static data** — like a signal, its points are already
  present, so pan/zoom re-windows the markers client-side with no resampling.
  Overlaid curves still resample adaptively over the same window.
- A bare `plot(scatter(...))` derives its window from the data's x-extent (with
  a little padding); give an explicit variable and window to frame it yourself.
- The hover probe **snaps to the nearest point** (2-D), reading off its exact
  `(x, y)`; a non-finite y is a gap, drawn as no marker.
- Points evaluate to f64 for drawing, like every plotted value — the data
  itself stays exact in the workspace.

## Scatterplot matrices

```
pairs(M)
pairs(M, [name1, name2, ...])
pairs(struct)
pairs(struct, [field1, field2, ...])
```

A **scatterplot matrix** (SPLOM) — the fastest way to eyeball a multivariate
dataset. Given k variables it draws a k×k grid of panels: the **lower triangle**
is the pairwise scatter, the **upper triangle** is the Pearson correlation
(coloured by sign), and the **diagonal** names each variable. Every panel in a
column shares one x-scale and every panel in a row shares one y-scale, so a
relationship reads consistently across the grid.

The data is a matrix whose **columns are variables** and rows are observations
(the same layout as [`stats.cormat`](stats.md#statscovmat-statscormat)):

```text
>> pairs([1, 2; 2, 4; 3, 6])               # default labels x1, x2, …
>> pairs(M, [mpg, weight, hp])             # name the columns
```

A [CSV import](../getting-started.md) lands as a struct of named columns, so a
whole table plots in one call — `pairs` uses the struct's numeric columns and
labels them by field name. Non-numeric columns (a category like `origin`) are
skipped, the way a data-frame pair plot uses only the numeric columns:

```text
>> cars := struct(mpg = [...], weight = [...], origin = [us; eu; us; ...])
>> pairs(cars)                              # panels for mpg & weight
```

To plot only **some** of a struct's fields, name them in a second list — they're
selected (and ordered) by name, and column names stay symbolic, so a workspace
binding of `weight` won't disturb the column called `weight`:

```text
>> pairs(cars, [mpg, weight])              # just these two, in this order
```

To read off the exact numbers behind the picture, pair it with
[`stats.cormat`](stats.md#statscovmat-statscormat) (`N(stats.cormat(M))`).

In the web app:

- The grid is **static** (a SPLOM is for reading, not panning); export it with
  the **png** button or in a PDF, labels and correlations included.
- Dense data is **decimated** by an even stride to keep the panels responsive;
  the caption notes when it's showing a thinned view.
- Needs 2–10 variables; beyond ten the panels are too small to read (an error
  asks you to select fewer columns).

## `plot3d`

```
plot3d(f, x, a, b, y, c, d)
```

The surface z = f(x, y) over `[a, b]` × `[c, d]`. The two plot variables
must differ.

```text
>> plot3d(x*y, x, -1, 1, y, -1, 1)
plot3d(x*y, x, -1, 1, y, -1, 1)
```

Both variables are taken by name and shadowed while `f` evaluates, exactly
as in `plot`.

In the web app:

- The sampling grid is **adaptive**: 81×81 for smooth surfaces, refining up
  to 641×641 while the grid fails a convergence test (each sample is checked
  against linear interpolation of its 2×-coarser neighbors — if the two
  disagree, the function has structure between the samples and the grid
  doubles). Oscillatory surfaces like `sin(x*y)` over a wide window get the
  resolution they need instead of aliasing into spikes.
- When even the finest grid can't certify a window, the plot is labeled
  **⚠ undersampled** rather than silently drawn wrong — fine structure may
  be aliased there, and zooming in (which resamples adaptively) clears it.
- **Drag rotates** the camera; **shift+drag pans** the domain and the **wheel
  zooms** it about the cursor. Both domain moves resample through the same
  adaptive policy as the 2D plot's pan/zoom; alt+wheel dollies the camera
  instead of touching the window.

## `scatter3d`

```
scatter3d(x, y, z)
```

The 3D sibling of [`scatter`](#scatter): three equal-length vectors drawn as
markers in the surface view. `scatter3d` is a data value, not a plot on its
own; `plot3d` draws it, alone or overlaid on a surface:

```text
>> plot3d(scatter3d(xs, ys, zs))                      # a point cloud, auto-boxed
>> plot3d(b0 + b1*x + b2*y, scatter3d(xs, ys, zs), x, 0, 10, y, 0, 10)
```

The overlay form is the natural way to check a fitted surface against the data
it was fit to — the markers and the surface share one box, just as a 2D
`scatter` overlays a curve.

In the web app:

- Markers are **static data** — they orbit, dolly, pan and zoom with the box
  but never resample. A bare `plot3d(scatter3d(...))` boxes the view from the
  data's x/y-extent (with padding); the z-range covers the points (and the
  surface, when overlaid).
- The hover probe **snaps to the nearest marker** and reads off its exact
  `(x, y, z)`; markers behind the surface are occluded, as depth expects.
- Points with a non-finite coordinate are dropped. Coordinates evaluate to f64
  for drawing, like every plotted value — the data stays exact in the
  workspace.

## Spectrograms

```
spectrogram(s)
spectrogram(s, nfft)
spectrogram(s, nfft, hop)
```

The STFT heatmap of a signal: time (samples) across, frequency (in units of
π rad/sample) up, magnitude in dB — periodic Hann window, `nfft` a power of
two (default fitted to the signal, hop = nfft/4). Real signals show the
one-sided spectrum [0, π]; complex (I/Q) signals show the full centered
spectrum [−π, π].

Like every plot, the picture is display-path (f64 midpoints, max-pooled to
the screen grid — the caption notes when pooling engaged); the exact
counterpart for any single frame is [`dsp.stft`](dsp.md#dspstft) or a
windowed `dsp.fft` of a `slice`.
