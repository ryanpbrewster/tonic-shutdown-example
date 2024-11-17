# Graceful shutdown

You want your server to shut down gracefully:
- stop accepting new connections
- notify existing connections that they should stop sending new requests
- wait for any live streams on existing connections to wrap up


`serve_with_shutdown` is the relevant method. But what happens if the live streams _don't_
wrap up in a timely fashion?

## Example of blocking a graceful shutdown

In one terminal, run the server:
```
$ cargo run
   Compiling tonic-shutdown-example v0.1.0 (/Users/ryanpbrewster/Library/Developer/rust/tonic-shutdown-example)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.39s
     Running `target/debug/tonic-shutdown-example`
2024-11-17T01:04:38.219002Z  INFO tonic_shutdown_example: server listening on [::]:50051
```

In another terminal, open a gRPC stream. One easy way is to use [grpcurl](https://github.com/fullstorydev/grpcurl)
```
$ grpcurl --plaintext localhost:50051 grpc.health.v1.Health.Watch
{
  "status": "SERVING"
}
```

Now we have a stream open, and it will prevent graceful shutdown when we attempt to stop the server. In the first terminal, send a SIGINT (usually via `Ctrl+C`):
```
^C2024-11-17T01:04:41.097180Z  INFO tonic_shutdown_example: waiting forever for clients to disconnect
2024-11-17T01:04:41.097226Z  INFO tonic_shutdown_example: shutting down server, trying to drain traffic
```

This will block forever and the server will never be able to exit on its own.

## Options

The brute-force option here is to send a `SIGKILL` to the server process. I dislike this option, because:
- The server does not necessarily get an opportunity to perform cleanup operations
- I usually like to monitor my live services for `SIGKILL` because that usually indicates that something is very wrong

So option 2 is to give up on a proper graceful shutdown and instead do a
_semi_-graceful shutdown: give live streams some amount of time to wrap up, then
forcefully interrupt them.

## Example semi-graceful shutdown

In one terminal, start the server, but tell it to use a finite grace period:
```
$ cargo run -- --grace-period-ms=5000
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.13s
     Running `target/debug/tonic-shutdown-example --grace-period-ms 5000`
2024-11-17T01:05:02.707329Z  INFO tonic_shutdown_example: server listening on [::]:50051
```

In a second terminal, open up the stream:
```
$ grpcurl --plaintext localhost:50051 grpc.health.v1.Health.Watch
{
  "status": "SERVING"
}
```

Now send a SIGINT to the server. After 5s, it will give up on the live stream and interrupt it
```
^C2024-11-17T01:05:06.262489Z  INFO tonic_shutdown_example: shutting down server, trying to drain traffic
2024-11-17T01:05:06.262581Z  INFO tonic_shutdown_example: waiting up to 5000ms for clients to disconnect
2024-11-17T01:05:11.264676Z  WARN tonic_shutdown_example: grace period exhausted, forcefully shutting down
```

And from the client side this looks like
```
{
  "status": "NOT_SERVING"
}
ERROR:
  Code: Unavailable
  Message: closing transport due to: connection error: desc = "error reading from server: EOF", received prior goaway: code: NO_ERROR
```