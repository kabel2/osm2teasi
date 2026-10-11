# Download page and browser installer

What serves the zips of `tools/build_world.sh`: nginx in a container, with https and one
bandwidth cap shared by all downloads.

```
html/index.html           the list of maps, read from maps/report.tsv
html/install/             the installer: copies a map onto the device from the browser
nginx.conf                https, /maps/ from the build output
shape.sh                  caps what the container sends (cake, RATE=10mbit)
reload.sh                 reloads nginx daily, for the renewed certificate
test.js                   the installer against a fake device, in node
```

Put the output of `build_world.sh` (report.tsv and the continent folders) into `./maps`,
your domain into `nginx.conf` and the certificate's directory into `compose.yaml`, then
`docker compose up -d --build`.

The installer needs Chrome or Edge (File System Access API) and https. It reads the zip's
directory with one range request, then each map file by its offset: written straight onto
the device and checked against its CRC. It deletes the older files of the same country and
puts the new files into `BikeNav/packages.xml` where an original map was listed (the first
time it keeps the old one as `packages.orig.xml`). Without a server it works too: a zip
downloaded beforehand can be picked from the disk.

Big countries take a while at 10 Mbit/s, so it holds a wake lock while the tab is in front,
and when the connection breaks — or the computer went to sleep anyway — it goes on from the
last byte with another range request (`If-Range` with the ETag, so a zip replaced meanwhile
is noticed). The files that are complete are remembered in `localStorage`; choosing the same
map again after the tab was closed goes on with the next file. `node test.js <zip>` runs
all of this against an in-memory device and a connection that keeps breaking.
