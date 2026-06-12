# Constants

`pi`, `e`, and `I` are **ordinary names that user bindings shadow** — they
mean the constants only while unbound, so `e` and `i` stay free for loop
counters and the like. `true`/`false` are literals and can never be assigned.

## `pi`

The circle constant π, as a symbolic object (also writable as `π`).

```text
>> 2*pi + pi
3*π
>> N(pi, 50)
3.1415926535897932384626433832795028841971693993751
```

## `e`

Euler's number, as a symbolic object.

```text
>> N(exp(1), 10)         # e the value
2.718281828
>> e := 5                # …but e is shadowable, like any name
5
```

(Note `3e5` is *not* scientific notation — it is rejected with a hint to
write `3*10^5`.)

## `I`

The imaginary unit (capital). See [Complex numbers](../language/complex.md).

```text
>> (1 + I)^2
2*I
>> sqrt(-4)
2*I
```

## `true` / `false`

The boolean literals — produced by comparisons and `and`/`or`/`not`,
consumed by `if`/`while`. Unassignable:

```text
>> true := 1
error: cannot assign to 'true'
```

Booleans are opaque to arithmetic; `true + 1` is an error.
