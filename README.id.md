<p align="center">
  <img src="docs/assets/banner.svg" alt="ChordKu: lagu apa pun jadi chart chord, di laptop sendiri" width="100%">
</p>

<p align="center">
  <img alt="Rust" src="https://img.shields.io/badge/server-Rust-dea584?logo=rust&logoColor=black">
  <img alt="Svelte" src="https://img.shields.io/badge/UI-Svelte%205-ff3e00?logo=svelte&logoColor=white">
  <img alt="ONNX Runtime" src="https://img.shields.io/badge/inference-ONNX%20Runtime-7a7a7a?logo=onnx&logoColor=white">
  <img alt="CUDA" src="https://img.shields.io/badge/GPU-CUDA%20%C2%B7%20DirectML-76b900?logo=nvidia&logoColor=white">
  <img alt="Platform" src="https://img.shields.io/badge/host-Windows%2011-0078d4?logo=windows11&logoColor=white">
  <img alt="License" src="https://img.shields.io/badge/license-MIT-blue">
</p>

<p align="center">
  <a href="README.md">English</a> · <b>Bahasa Indonesia</b>
</p>

---

ChordKu mengubah **lagu apa pun menjadi chart chord**. Tempel tautan YouTube (atau situs lain
yang didukung yt-dlp), pilih file audio, atau rekam lewat mikrofon. Beberapa detik kemudian muncul
chart chord per bar yang berjalan mengikuti lagu, di nada dasar yang ingin Anda mainkan.

Semua berjalan lokal: satu binary Rust dengan ONNX Runtime di dalamnya. Tanpa akun, tanpa cloud,
tanpa Python saat dijalankan. Antarmukanya berbahasa Indonesia.

## Fitur utama

- **Chord, ketukan, kunci, bagian lagu.** Pengenal chord 170 kelas (mayor, minor, 7, maj7, m7,
  sus, dim, aug, 6, dan lainnya) yang dipaskan ke garis bar. Ada koreksi tuning untuk rekaman
  yang tidak tepat di A = 440 Hz, dan deteksi naik nada dasar di reff terakhir.
- **Transpose dan capo.** Naik atau turunkan nada dasar (audionya ikut digeser), dapatkan saran
  posisi capo berdasarkan bentuk chord yang paling mudah, ganti ke angka Nashville, sederhanakan
  chord.
- **Pemutar sinkron.** Ketukan yang sedang berbunyi menyala. Ulang satu bagian atau rentang A-B,
  perlambat sampai 50 % tanpa mengubah nada.
- **Diagram chord mengambang.** Panel contekan yang bisa diseret dan tetap terlihat saat chart
  digulir: gitar, ukulele, piano, atau ketiganya, untuk chord sekarang dan berikutnya.
- **Pisah vokal dan instrumen** (opsional). Vokal, drum, bass, lainnya. Bisukan penyanyi,
  solo-kan bass.
- **Koreksi hasil mesin.** Ketuk chord, pilih salah satu alternatif dari pengenal atau chord
  lain. Koreksi tetap ada walau lagu dianalisis ulang.
- **Pustaka.** Cari, label, favorit, setlist; ekspor ke ChordPro, teks, atau PDF.
- **Dari HP.** Mode HTTPS di Wi-Fi rumah (harus diaktifkan dulu) dengan pemasangan lewat kode QR.
- **Bagikan setlist.** Satu halaman web mandiri per setlist (chord saja, tanpa audio): kirim lewat
  chat atau unggah ke GitHub Pages. Semua orang membukanya di HP masing-masing, memilih transpose
  dan capo sendiri, dan chart mengikuti video YouTube yang disematkan.

<p align="center">
  <img src="docs/assets/app.svg" alt="Halaman lagu: bilah alat, chart chord sinkron, diagram chord mengambang, mixer stem, dan transport" width="100%">
</p>

## Cara kerja

<p align="center">
  <img src="docs/assets/pipeline.svg" alt="Pipeline: sumber, decode, jalur analisis cepat, jalur pisah stem, pemutar di browser" width="100%">
</p>

Dua jalur berbagi satu thread inference. Jalur cepat (tuning, ketukan, chord, chart) selesai dalam
hitungan detik dan selalu didahulukan; pemisahan stem berjalan di latar dan mengalah di setiap
batas segmen. Rincian dan alasan tiap keputusan: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## Seberapa cepat, seberapa akurat

<p align="center">
  <img src="docs/assets/performance.svg" alt="Hasil pengukuran kecepatan dan akurasi" width="100%">
</p>

Diukur di laptop Core Ultra 5 125H dengan RTX 3050:

| | |
|---|---|
| Lagu 4 menit, dari unggah sampai chart tampil | **4,1 detik** (sekitar 7 detik bila hanya CPU) |
| Video 10 menit, hanya CPU | sekitar 17 detik |
| Pisah stem, GPU NVIDIA (CUDA, presisi setengah) | sekitar **12 detik** untuk lagu 4 menit |
| Pisah stem, hanya CPU | 0,30 x durasi lagu, memori puncak 2,5 GB, berjalan di latar |

Akurasi (weighted chord symbol recall) pada 91 menit rekaman gitar asli (GuitarSet, 180 rekaman
iringan, lihat [docs/ACCURACY.md](docs/ACCURACY.md)): **akar chord 82,9 %, mayor/minor 79,2 %,
chord tujuh 67,6 %** secara keseluruhan; **90-92 % mayor/minor** untuk gaya rock dan
singer-songwriter, 60-69 % untuk funk dan jazz. Artinya chart benar di sebagian besar tempat dan
cepat dikoreksi di tempat yang salah.

## Setiap prosesor diukur

<p align="center">
  <img src="docs/assets/acceleration.svg" alt="Rencana perangkat: tiap model diukur di CPU, GPU NVIDIA, GPU Intel, dan NPU" width="100%">
</p>

ChordKu mengukur sendiri tiap model di CPU, GPU NVIDIA, GPU Intel, dan NPU, lalu hanya memakai
perangkat yang terbukti lebih cepat **dan** hasilnya sama dengan CPU. Di laptop pengembangan:
ketukan dan pisah stem di RTX 3050, chord di CPU, NPU tidak dipakai karena terukur tidak lebih
cepat atau hasilnya salah. Runtime CUDA (sekitar 1,5 GB) diunduh dari dalam aplikasi dengan
SHA-256 yang dikunci. Tabel lengkap dan alasannya: [docs/ACCELERATION.md](docs/ACCELERATION.md).

## Menjalankan (Windows 11)

Butuh: Rust (MSVC), Visual Studio Build Tools dengan komponen CMake, Node.js 22+, Python 3.11
(hanya untuk setup), Git.

```powershell
git clone https://github.com/yoelkh/chordku.git
cd chordku
tools\setup.ps1      # sekali saja: export model chord, build UI
tools\run.ps1        # build lalu buka http://127.0.0.1:7464
```

Saat pertama dipakai, buka **Pengaturan > Model** lalu unduh pelacak ketukan (11 MB, wajib) dan,
bila ingin memisah stem, model pemisah (171 MB). yt-dlp dipasang otomatis dengan checksum
terverifikasi saat tautan pertama ditambahkan; untuk YouTube perlu Node.js 22+ atau Deno di PATH.

`tools\build-release.ps1` membuat folder portabel `dist\ChordKu`.

Variabel lingkungan: `CHORDKU_PORT` (7464), `CHORDKU_DATA` (bawaan `%LOCALAPPDATA%\ChordKu\data`),
`CHORDKU_MODELS`, `CHORDKU_THREADS`, `CHORDKU_MAX_SECONDS` (1200), `CHORDKU_MAX_UPLOAD_MB` (300),
`CHORDKU_MIN_FREE_MB` (1536), `CHORDKU_CUDA`, `CHORDKU_NO_ACCEL`, `CHORDKU_NO_HALF`.

## Tips pemakaian

- **Diagram mengambang**: seret panel dari bagian atasnya, pilih Gitar, Ukulele, Piano, atau
  Semua (ketiganya sekaligus), ketuk diagram untuk posisi jari lain, ciutkan jadi satu baris
  (chord sekarang dan berikutnya). Di HP panel mulai dalam keadaan ciut. Tombolnya ada di bilah
  alat halaman lagu.
- Tombol **Koreksi chord** lalu ketuk chord yang salah. Ketuk ketukan kosong di tengah bar untuk
  memulai chord baru di situ.
- Bila hitungan "satu" meleset, pakai **Lainnya > Geser hitungan "satu"**. Bila tempo terbaca dua
  kali lipat atau setengahnya, ada pilihannya di menu yang sama. Birama 3/4 atau 6/8 juga bisa diatur.
- Pintasan: spasi (putar/jeda), panah kiri/kanan (5 detik), `[` dan `]` (transpose).
- Rekaman mikrofon paling akurat bila 20 detik atau lebih dan tanpa banyak suara obrolan.

## Bagikan setlist

Di **Pustaka**, buka sebuah setlist lalu tekan **Bagikan (unduh halaman web)**. Hasil unduhannya
satu file HTML berisi chart, koreksi, dan diagram semua lagu di setlist itu, tanpa yang lain: tanpa
audio, tanpa server. Bisa dibuka dari pengelola file HP, dari lampiran chat, atau dari hosting
statis mana pun. Pilihan transpose, capo, dan notasi tiap pembaca tersimpan di browsernya sendiri.

Lagu yang berasal dari YouTube mendapat sematan videonya (perlu internet): sorotan ketukan dan
gulir otomatis mengikuti video, ketuk sel untuk melompat, bagian dan rentang A-B mengulang video.
Lagu lain berjalan dengan penghitung tempo tanpa suara. Layar tetap menyala selama lagu terbuka.

Untuk menerbitkan ke GitHub Pages (gratis, publik):

```powershell
tools\publish-setlist.ps1 "$HOME\Downloads\ibadah-minggu.html"
```

Jalankan pertama membuat repositori publik `<akun>/chordku-setlists` dan mengaktifkan Pages;
jalankan berikutnya menambah atau mengganti satu halaman dan mencetak tautannya
(`https://<akun>.github.io/chordku-setlists/<nama>/`). `-Remove <nama>` menurunkan halaman.

## Baris perintah

```
chordku-cli analyze  lagu.mp3 [--json out.json]    kunci, tempo, bagian, bar
chordku-cli chords   lagu.mp3 [--lab out.lab]      label chord per frame
chordku-cli beats    lagu.mp3                      ketukan dan downbeat
chordku-cli separate lagu.mp3 folder_keluaran      empat stem WAV
chordku-cli eval     folder_audio_dan_lab          WCSR terhadap anotasi
chordku-cli bench    lagu.mp3                      kecepatan dan ketepatan per model dan perangkat
```

## Pengujian

```
cargo test --workspace        # 50+ unit test, property test, dan uji paritas golden
cd web; npm test              # teori, edit chart, ekspor (vitest + fast-check)
cd web; npm run check         # svelte-check
node tools/lan-check.mjs <ip>:7465 <kode>    # mode LAN lewat HTTP/2, seperti HP
```

## Dokumentasi

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): pipeline, crate, alasan tiap keputusan, model keamanan
- [docs/MODELS.md](docs/MODELS.md): model yang dipakai, alasannya, lisensi, export, uji paritas
- [docs/ACCURACY.md](docs/ACCURACY.md): hasil pengukuran dan cara mengukur sendiri
- [docs/ACCELERATION.md](docs/ACCELERATION.md): pengukuran CPU, GPU, NPU, dan rencana perangkat
- [docs/LAN.md](docs/LAN.md): akses dari HP, sertifikat, pemasangan
- [THIRD_PARTY.md](THIRD_PARTY.md): kredit dan lisensi

## Pemakaian wajar

ChordKu adalah alat latihan. Unduh hanya materi yang boleh Anda unduh, untuk dipelajari sendiri.
Bobot model pemisah stem disediakan pembuatnya untuk riset dan pemakaian pribadi; ChordKu tidak
menyebarkannya.

[Lisensi MIT](LICENSE).
