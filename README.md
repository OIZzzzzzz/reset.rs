# reset.rs

Add reset abstraction(rust for linux), include new, register, ResetDriverOps(reset/assert/deassert/status).

## Clone repo

In your linux directory, clone this project

```shell
cd `path to your kernel`/rust/kernel
git clone git@github.com:OIZzzzzzz/reset.rs.git
```

## Linux support Cargo 

The cross-kernel driver framework follows a componentized design and uses cargo to resolve component dependencies,
so it is necessary to add R4L support for cargo construction.
reference link: https://github.com/guoweikang/osl


## Add other for reset.rs

Add this line into `path to your kernel`/rust/kernel/lib.rs

``` shell
pub mod reset;
```


