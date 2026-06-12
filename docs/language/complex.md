# Complex numbers

`I` (capital — so `i` stays free for loop counters) is the imaginary unit.
Complex numbers behave like *numbers*: arithmetic folds eagerly rather than
staying factored.

```text
>> (1 + I)*(1 - I)
2
>> (1 + I)^2
2*I
>> sqrt(-4)                 # square roots of negatives are imaginary
2*I
>> (2 + 3*I)/(1 + I)
5/2 + 1/2*I
```

## Parts, conjugates, modulus

```text
>> conj(2 + 3*I)
2 - 3*I
>> re(2 + 3*I)
2
>> im(2 + 3*I)
3
>> abs(3 + 4*I)
5
>> (2 + 3*I) * conj(2 + 3*I)   # = |z|²
13
```

Real and imaginary parts may themselves be symbolic (`x + I` is fine).
**Symbols are assumed real**, so `conj(x) = x`. See the
[complex reference](../reference/complex.md) for each function.

Complex values flow through everything else — eigenvalues, calculus:

```text
>> eigenvalues([1,-1; 1,1])    # complex eigenvalues, returned not refused
[ 1 + I ]
[ 1 - I ]
>> diff(x^2 + I*x, x)
2*x + I
```

## Complex transcendentals

`exp`/`sin`/`cos`/`tan`/`ln` of a complex argument evaluate numerically via
[`N`](../reference/numeric.md#n), to arbitrary precision, through Euler's
formula and the hyperbolic identities; complex powers go via `exp(w·ln z)`:

```text
>> N(exp(I*pi))                 # Euler's identity, recovered exactly
-1
>> N(exp(2*pi*I/3), 25)         # a primitive cube root of unity
-0.5 + 0.8660254037844386467637232*I
>> N(ln(I), 20)                 # ln(i) = iπ/2
1.5707963267948966192*I
```

### Residue snapping — and its honest limits

`N(exp(I*pi) + 1)` shows a ~1e-60·i residue rather than `0` — the honest
precision floor of a numeric computation, since proving it's exactly zero is
undecidable.

The snapping that makes `N(exp(I*pi))` read `-1` applies **only to
transcendental results**, where a component that is mathematically zero can
only come back as cancellation residue. A purely arithmetic complex value has
full relative precision in each component, so a genuinely tiny part survives:
`N(1 + 10^(-50)*I, 30)` is `1 + 1e-50*I`, not `1`. (The remaining caveat: a
tiny *exact* component fed through a transcendental expression — say
`1 + sin(10^(-50))*I` — is indistinguishable from residue and gets snapped.)
