# Calculus and symbolic manipulation

## `diff` / `D`

```
diff(expr, x)
D(expr, x)
```

The derivative of `expr` with respect to `x`. `D` is an alias.

```text
>> diff(sin(x), x)
cos(x)
>> diff(x^3 + 2*x, x)
2 + 3*x^2
>> diff(x*sin(x), x)         # product rule
sin(x) + x*cos(x)
>> diff(sin(x^2), x)         # chain rule
2*x*cos(x^2)
>> diff(diff(x^4, x), x)     # nest for higher derivatives
12*x^2
>> diff([x^2, sin(x); x, 1], x)    # distributes entrywise over matrices
[ 2*x  cos(x) ]
[   1       0 ]
```

### The variable is taken by *name*

`diff` sees its first argument **before** the workspace collapses the
variable, then substitutes any binding back into the derivative. So a bound
`x` doesn't break differentiation — it evaluates the derivative at that
point:

```text
>> x := 3
3
>> diff(x^2, x)        # the derivative 2x, evaluated at x = 3 — not diff(9, 3)
6
```

The second argument is usually a bare identifier; anything that evaluates to
a symbol also works.

## `subs`

```
subs(expr, x, val)
```

Substitute `val` for the variable `x` in `expr`. Like `diff`, the variable
is taken by name, so a workspace binding of `x` doesn't collapse `expr`
first.

```text
>> subs(x^2 + x, x, 3)
12
>> subs(x^2 + y, x, y)
y + y^2
>> subs(sin(x) + x^2, x, pi)
π^2 + sin(π)
>> subs(diff(x^3, x), x, 2)      # evaluate a derivative at a point
12
```

## `expand`

```
expand(expr)
```

Expand products and integer powers, then re-canonicalize (which combines like
terms). This is the explicit counterpart to the bounded simplification that
happens automatically at construction.

```text
>> expand((x+1)^2)
1 + x^2 + 2*x
>> expand((x+1)^3)
1 + x^3 + 3*x + 3*x^2
>> expand((x+y)*(x-y))
x^2 - y^2
```

A common use is deciding structural equality of polynomials — `==` compares
canonical *forms*, so factored and expanded forms differ until you expand:

```text
>> (x-1)*(x+1) == x^2 - 1
false
>> expand((x-1)*(x+1)) == x^2 - 1
true
```
