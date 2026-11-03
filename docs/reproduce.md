# Reproduce the IRIS Wasm FDW query locally

This walkthrough uses disposable Docker containers and synthetic data to exercise PostgreSQL → Supabase Wrappers → Wasm FDW → IRIS Atelier API. It does not require a hosted Supabase project. Run it in a shell on macOS or Linux with Docker and internet access; the images must be available for your machine's architecture. The examples use IRIS for Health Community 2026.2 and Supabase Postgres 17.6.1.136, with Wrappers 0.6.1.

The IRIS setup below enables Basic authentication and sets a demo password on `_SYSTEM`. **Use it only in the disposable IRIS container created here**, never in a business instance. No container ports are published. Do not use these credentials outside this isolated example.

## 1. Obtain the Wasm component

Download `iris_fdw.wasm` and `checksum.txt` from the [v0.1.0 release](https://github.com/boanerges1227/iris-wasm-fdw/releases/tag/v0.1.0) into a new empty directory. Verify the file before use:

```sh
sha256sum -c checksum.txt
```

On macOS, use `shasum -a 256 -c checksum.txt` instead. Alternatively, build the component as described in the [README](../README.md), and use the built file in place of the downloaded artifact.

## 2. Start isolated containers

Run the following from the directory containing `iris_fdw.wasm`. The names are unique to this shell; retain the variable values for the remaining steps.

```sh
fdw_demo_id="iris-fdw-demo-$(date +%s)-$$"
fdw_demo_network="$fdw_demo_id-net"
fdw_demo_iris="$fdw_demo_id-iris"
fdw_demo_db="$fdw_demo_id-db"
docker network create "$fdw_demo_network"
docker run -d --name "$fdw_demo_iris" --network "$fdw_demo_network" \
  intersystems/irishealth-community:2026.2 --check-caps false
docker run -d --name "$fdw_demo_db" --network "$fdw_demo_network" \
  -e POSTGRES_PASSWORD=isolated-test-password supabase/postgres:17.6.1.136
```

Wait until IRIS is ready (the first startup may take a few minutes):

```sh
until docker exec -i "$fdw_demo_iris" iris session IRIS -U USER </dev/null >/dev/null 2>&1; do sleep 2; done
```

## 3. Configure the disposable IRIS and add sample rows

```sh
docker exec -i "$fdw_demo_iris" iris session IRIS -U %SYS <<'IRIS'
zn "%SYS"
set props("Password")="SYS",props("Enabled")=1
set sc=##class(Security.Users).Modify("_SYSTEM",.props)
write "USER_SETUP_OK=",'$SYSTEM.Status.IsError(sc),!
kill props
set props("AutheEnabled")=32
set sc=##class(Security.Applications).Modify("/api/atelier",.props)
write "APP_SETUP_OK=",'$SYSTEM.Status.IsError(sc),!
halt
IRIS
```

Both output values must be `1`. Create a small synthetic table:

```sh
docker exec -i "$fdw_demo_iris" iris session IRIS -U USER <<'IRIS'
set r=##class(%SQL.Statement).%ExecDirect(,"CREATE TABLE FdwTest.Person (Name VARCHAR(100), CreatedAt VARCHAR(40))")
write "CREATE SQLCODE=",r.%SQLCODE," ",r.%Message,!
set r=##class(%SQL.Statement).%ExecDirect(,"INSERT INTO FdwTest.Person VALUES ('Alice', '2026-10-31T01:02:03Z')")
write "INSERT SQLCODE=",r.%SQLCODE," ",r.%Message,!
set r=##class(%SQL.Statement).%ExecDirect(,"INSERT INTO FdwTest.Person VALUES ('Bob', NULL)")
write "INSERT SQLCODE=",r.%SQLCODE," ",r.%Message,!
halt
IRIS
```

All three SQLCODE values must be `0`. These commands are for a fresh disposable instance, not an existing fixture.

## 4. Query IRIS from PostgreSQL

Wait for PostgreSQL, install Wrappers, and copy the component into the PostgreSQL container:

```sh
until docker exec -e PGPASSWORD=isolated-test-password "$fdw_demo_db" psql -h 127.0.0.1 -U postgres -d postgres -v ON_ERROR_STOP=1 -c 'CREATE EXTENSION IF NOT EXISTS wrappers;' >/dev/null 2>&1; do sleep 2; done
docker cp iris_fdw.wasm "$fdw_demo_db:/tmp/iris_fdw.wasm"
docker exec "$fdw_demo_db" chmod a+r /tmp/iris_fdw.wasm
```

The local `file://` URL below is for this Docker demonstration. A hosted Supabase deployment needs an HTTPS package URL and checksum, plus network access to the IRIS API.

```sh
docker exec -i "$fdw_demo_db" psql -U postgres -d postgres -v ON_ERROR_STOP=1 \
  -v "iris_api_url=http://$fdw_demo_iris:52773/api/atelier/v3" <<'SQL'
BEGIN;
CREATE FOREIGN DATA WRAPPER iris_demo_wrapper
  HANDLER wasm_fdw_handler VALIDATOR wasm_fdw_validator;
CREATE SERVER iris_demo_server FOREIGN DATA WRAPPER iris_demo_wrapper OPTIONS (
  fdw_package_url 'file:///tmp/iris_fdw.wasm',
  fdw_package_name 'intersystems:iris-fdw', fdw_package_version '0.1.0',
  api_url :'iris_api_url', namespace 'USER', username '_SYSTEM', password 'SYS'
);
CREATE FOREIGN TABLE iris_demo_people (name text, created_at timestamp)
  SERVER iris_demo_server
  OPTIONS (query 'SELECT Name AS name, CreatedAt AS created_at FROM FdwTest.Person ORDER BY Name');
SELECT * FROM iris_demo_people;
SELECT * FROM iris_demo_people;
ROLLBACK;
SQL
```

Both queries should return Alice with timestamp `2026-10-31 01:02:03` and Bob with a SQL NULL timestamp. The transaction rolls back the PostgreSQL objects. Independent repeated queries are supported; host-driven rescans are not.

## 5. Clean up

These commands delete only the two containers and network created by this walkthrough; use the original variable values from step 2.

```sh
docker rm -fv "$fdw_demo_db" "$fdw_demo_iris"
docker network rm "$fdw_demo_network"
```

For actual deployments, configure a restricted read-only IRIS account, use HTTPS and follow the limitations in the [README](../README.md). The example demonstrates a bounded query rather than pagination, write support, or hosted Supabase compatibility.
