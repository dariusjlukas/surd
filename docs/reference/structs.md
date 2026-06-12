# Structs

## `struct`

```
struct(name = value, ...)
```

Build a struct from named fields. Each argument must literally be
`name = value` — the names are read from the **syntax** (a workspace binding
for `gain` elsewhere doesn't interfere); the values evaluate normally.

```text
>> s := struct(gain = 1/3, taps = [1, 2; 3, 4])
struct(gain = 1/3, taps = [ 1  2 ]
[ 3  4 ])
```

Fields are read with `.`, which binds tighter than `^`:

```text
>> s.gain
1/3
>> s.gain * det(s.taps)
-2/3
```

Rules:

- Fields hold **anything a variable can** — numbers, symbols, matrices,
  functions, other structs.
- Fields are kept **sorted by name**, so `==` on structs is
  field-order-independent.
- Structs are **opaque to arithmetic** — `s + 1` is an error, not a guess.
- Reading a missing field is an error that lists the available fields.
- An argument that isn't `name = value` is an error:
  `struct expects 'name = value' arguments, e.g. struct(a = 1)`.

Structs are also how several built-ins return multiple values
([`lu`](linear-algebra.md#lu), [`qr`](linear-algebra.md#qr), underdetermined
[`solve`](linear-algebra.md#solve)), and how the web app's data imports
deliver CSV columns and JSON keys — see
[Structs](../language/structs.md) in the language guide.
