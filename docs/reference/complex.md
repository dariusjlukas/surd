# Complex numbers

The imaginary unit is the constant [`I`](constants.md#i) (capital, so `i`
stays free as an ordinary variable). Complex arithmetic folds eagerly —
`(1 + I)^2` is `2*I`, not a factored expression. Background and
transcendentals: [Complex numbers](../language/complex.md).

**Symbols are assumed real** throughout: `conj(x) = x`, `re(x) = x`,
`im(x) = 0`.

## `conj`

```
conj(z)
```

The complex conjugate.

```text
>> conj(2 + 3*I)
2 - 3*I
>> conj(x)                     # symbols are assumed real
x
>> (2 + 3*I) * conj(2 + 3*I)   # = |z|²
13
```

## `re` / `real`

```
re(z)
real(z)
```

The real part. `real` is an alias.

```text
>> re(2 + 3*I)
2
>> re(x + 2*I)         # x assumed real
x
```

## `im` / `imag`

```
im(z)
imag(z)
```

The imaginary part (a real number — the coefficient of `I`). `imag` is an
alias.

```text
>> im(2 + 3*I)
3
>> im(x + 2*I)
2
```

## `abs`

```
abs(z)
```

The modulus √(re² + im²). For real arguments this is the ordinary absolute
value — see [`abs`](elementary.md#abs).

```text
>> abs(3 + 4*I)
5
>> abs(-5)
5
```
