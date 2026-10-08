# Download page and browser installer

What serves the zips of `tools/build_world.sh`: nginx in a container, with https and one
bandwidth cap shared by all downloads.

```
html/index.html           the list of maps, read from maps/report.tsv
html/install/             the installer: copies a map onto the device from the browser
nginx.conf                https, /maps/ from the build output
shape.sh                  caps what the container sends (cake, RATE=10mbit)
reload.sh                 reloads nginx daily, for the renewed certificate
```

Put the output of `build_world.sh` (report.tsv and the continent folders) into `./maps`,
your domain into `nginx.conf` and the certificate's directory into `compose.yaml`, then
`docker compose up -d --build`.

The installer needs Chrome or Edge (File System Access API) and https. It reads the zip's
directory with one range request, then streams the zip once: each map file is written
straight onto the device and checked against its CRC. It deletes the older files of the
same country and puts the new files into `BikeNav/packages.xml` where an original map was
listed (the first time it keeps the old one as `packages.orig.xml`). Without a server it
works too: a zip downloaded beforehand can be picked from the disk.
