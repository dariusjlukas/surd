# Modules and namespaces

Domain toolkits live behind **namespaces**, so the global builtin set stays
small: `dsp.dft(v)` calls the DFT in the built-in
[`dsp` namespace](../reference/dsp.md) without claiming the bare names `dft`
or `conv` for everyone.

```text
>> dsp.dft([1; 2; 3; 4])
[       10 ]
[ -2 + 2*I ]
[       -2 ]
[ -2 - 2*I ]
```

The syntax is the same field-access dot used by [structs](structs.md),
followed by an argument list: `base.name(args)`.

## User modules are structs of functions

There is no separate module machinery to learn. A struct can hold function
values, and a struct field followed by `(...)` calls it — so a module is
just a struct of functions:

```text
>> twice(x) := 2*x
>> inc(x) := x + 1
>> mylib := struct(twice = twice, inc = inc)
>> mylib.inc(mylib.twice(3))
7
```

Calls through a module use the same machinery as plain calls: arity is
checked, recursion is depth-guarded, and errors name the function. A field
that doesn't hold a function is a clean error
(`field 'a' holds '5', which is not a function`).

## Shadowing

Namespaces follow the same rule as every other builtin: a user binding of
the same name shadows it.

```text
>> dsp := struct(dft = myfastdft)
>> dsp.dft(v)                      # now calls yours
```

Unbound and uncalled, a namespace name is still an ordinary symbol — `dsp`
on its own evaluates to `dsp`. Referencing a namespace function without
calling it points at the syntax:

```text
>> dsp.dft
error: 'dsp.dft' names a function in the built-in 'dsp' namespace — call it
with arguments: dsp.dft(...)
```

## Built-in namespaces

| Namespace | Contents |
| --- | --- |
| [`dsp`](../reference/dsp.md) | Exact digital signal processing: DFT, convolution |
| [`stats`](../reference/stats.md) | Exact statistics: mean through regression and distributions |
| [`data`](../reference/data.md) | Data preparation for models: standardize, dummy-encode, group-by |
