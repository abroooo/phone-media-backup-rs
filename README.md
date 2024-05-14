# phone-media-backup-rs

## Pitch

app to sync image from android phone to server

## 🔌 assumptions

## ✨ Features

- 💾 backup photos and videos
- 📁 automatic sorting into directory structure
- 🔎 exif data based sorting
- 📆 only backup new files

## ⚡️ Requirements
- sshfs
- ssh access to phone (https://www.galexander.org/software/simplesshd) 

## ⚙️ Configuration

## 🚀 Usage

## 🪓 TODO

- [x] mount fs
- [x] get start date via ssh
- [x] get list of files to backup
- [x] backup files
- [x] set marker
- [x] add config
- [ ] add cli interface
- [ ] add info option (stats about the source, last backup ...)
- [ ] add verify option (check that all files in source are present at destination)
- [ ] optionally delete files older than X days
- [ ] logging
