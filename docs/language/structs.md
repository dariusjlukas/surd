# Structs

Structs group named values. Fields hold anything a variable can — numbers,
matrices, functions — and are read with `.`, which binds tighter than `^`.

```text
>> s := struct(gain = 1/3, taps = [1, 2; 3, 4])
struct(gain = 1/3, taps = [ 1  2 ]
[ 3  4 ])
>> s.gain
1/3
>> s.gain * det(s.taps)
-2/3
```

Behavior worth knowing:

- **Field names come from the syntax.** In `struct(gain = 1/3)` the name
  `gain` is taken literally — a workspace binding for `gain` elsewhere
  doesn't interfere.
- **Fields are kept sorted**, so `==` on structs is field-order-independent.
- **Structs are opaque to arithmetic** — `s + 1` is an error, not a guess.
- Reading a missing field is an error that lists the fields that do exist.

## Structs from built-ins

Several built-ins return structs:

- [`lu(A)`](../reference/linear-algebra.md#lu) → `struct(L, U, P)`
- [`qr(A)`](../reference/linear-algebra.md#qr) → `struct(Q, R)`
- [`solve(A, b)`](../reference/linear-algebra.md#solve) on an underdetermined
  system → `struct(particular, nullspace)`

```text
>> f := lu([4,3;6,3])
>> f.L
[   1  0 ]
[ 3/2  1 ]
>> f.U
[ 4     3 ]
[ 0  -3/2 ]
```

## Structs from data imports

In the web app, imported files (CSV, JSON, `surd-data`) land in a struct, so
imported names can never collide with existing bindings — a CSV with `temp`
and `time` columns imported as `sensor` is read as `sensor.temp`,
`sensor.time`. Numbers are read from their literal text — `0.1` in a sensor
log becomes the exact rational 1/10, never an f64.
