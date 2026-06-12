# Numeric evaluation

The one place exact crosses to approximate — deliberately visible.

## `N`

```
N(x)
N(x, digits)
```

The numeric (floating-point) value of `x`, to `digits` significant digits.
Without `digits`, the default precision is used (30 unless changed with
[`precision`](#precision)). Floats are arbitrary-precision (via
`astro-float`), not f64.

```text
>> N(1/3)
0.333333333333333333333333333333
>> N(pi, 100)
3.141592653589793238462643383279502884197169399375105820974944592307816406286208998628034825342117068
>> N(sqrt(2), 60)
1.41421356237309504880168872420969807856967187537694807317668
>> N(sin(pi/6), 40)        # = 1/2, recovered numerically
0.5
```

Values that are exactly representable come back exact — `N(sqrt(2)^2)` is
the integer `2`, because `sqrt(2)^2` already folded to `2` before `N` saw it.

`N` maps entrywise over matrices:

```text
>> N([1/3, 1/7; 2/3, 1], 5)
[ 0.33333  0.14286 ]
[ 0.66667        1 ]
>> N(eigenvalues([1,1;1,0]), 30)
[   1.61803398874989484820458683437 ]
[ -0.618033988749894848204586834366 ]
```

Complex arguments evaluate through Euler's formula and the hyperbolic
identities, to full precision — see
[Complex numbers](../language/complex.md#complex-transcendentals).

### Floats compare exactly

A binary float *is* the rational m·2^k, so float-vs-exact comparisons are
decided losslessly on that value:

```text
>> N(2) == 2
true
>> N(1/3) == 1/3       # the float is genuinely not 1/3
false
```

This is also the standard way to decide a symbolic comparison:

```text
>> pi < 4
error: cannot order 'π' and '4'; both must be numbers (try N(...))
>> N(pi) < 4
true
```

## `precision`

```
precision()
precision(digits)
```

With no argument, returns the current default digit count for `N`. With one
argument, sets it (clamped to 1…100,000) and returns the new value.

```text
>> precision()
30
>> precision(10)
10
>> N(pi)
3.141592654
>> precision(30)
30
```

The default applies only when `N` is called without an explicit `digits`
argument.
