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
- Pan/zoom **resamples** through the same adaptive policy, exactly like the
  2D plot.
