-- Replace all placeholders and select a source table that exists in IRIS.
CREATE EXTENSION IF NOT EXISTS wrappers;
CREATE FOREIGN DATA WRAPPER wasm_wrapper
  HANDLER wasm_fdw_handler VALIDATOR wasm_fdw_validator;
CREATE SERVER iris_server FOREIGN DATA WRAPPER wasm_wrapper OPTIONS (
  fdw_package_url 'https://YOUR_HOST/iris_fdw.wasm',
  fdw_package_name 'intersystems:iris-fdw',
  fdw_package_version '0.1.0',
  fdw_package_checksum 'REPLACE_WITH_ACTUAL_SHA256',
  api_url 'https://YOUR_IRIS_HOST/api/atelier/v3',
  namespace 'USER', username 'YOUR_READ_ONLY_USER', password 'YOUR_PASSWORD'
);
CREATE FOREIGN TABLE iris_people (name text)
  SERVER iris_server
  OPTIONS (query 'SELECT TOP 100 Name AS name FROM Sample.Person');
SELECT * FROM iris_people;
