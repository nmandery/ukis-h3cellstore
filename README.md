# bamboo_h3

Python bindings to integrate clickhouse H3 databases with the python data-science world.

![](doc/img/bamboo_h3.png)

## Goals

1. Provide an integration with the widely known Python libraries.
2. Abstraction of most storage details of H3 data.
3. Enable and encourage parallelization.
4. Handling of compute-intensive tasks on the client instead of the DB servers as the 
   clients are far easier to scale.
5. Handle compute-intensive tasks in native code instead of Python.

# Usage

## Connecting to clickhouse

This library uses [clickhouse_rs], so all the connection options from [the documentation there](https://docs.rs/clickhouse-rs/1.0.0-alpha.1/clickhouse_rs/index.html#dns)
can be used. A few things to keep in mind:

* Always use the cheap `lz4` compression. This reduces the amount of data to be transfered over the network.
* The default `connection_timeout` is quite low for large amounts of geodata. You may want to increase that.


## Logging

This library uses rusts [log crate](https://docs.rs/log/0.4.6/log/) together with 
the [env_logger crate](https://docs.rs/env_logger/0.8.2/env_logger/). This means that logging to `stdout` can be
controlled via environment variables. Set the `RUST_LOG` variable to `debug`, `error`, `info`, `warn`, or `trace` for the corresponding 
log output. This will give you log messages from all  libraries used, most of them will be from `clickhouse_rs`. To just get
the messages from `bamboo_h3` use:

```
RUST_LOG=bamboo_h3=debug python my-script.py
```

For more fine-grained logging settings, see the documentation of `env_logger`.

# other relevant libraries

* [offical h3 bindings](https://github.com/uber/h3-py)
* [h3ronpy](https://github.com/nmandery/h3ron/tree/master/h3ronpy)
