package main

import (
	"bufio"
	"fmt"
	"os"
	"sort"
	"strings"
	"time"
)

// ---------- Query set ----------

var defaultQueries = []string{
	"Permohonan cuti tahunan pegawai",
	"Pengadaan laptop untuk unit kerja",
	"Laporan keuangan triwulan III",
	"Usulan kenaikan pangkat pegawai",
	"Surat undangan rapat koordinasi pimpinan",
	"Laporan realisasi anggaran belanja modal",
	"Permohonan surat keterangan aktif pegawai",
	"Berita acara serah terima aset daerah",
	"Surat perintah perjalanan dinas luar daerah",
	"Pemberian penghargaan pegawai teladan",
	"Rekrutmen dan seleksi calon pegawai negeri",
	"Pengadaan barang dan jasa kantor",
	"Laporan kegiatan bimbingan teknis pengelolaan arsip",
	"Surat permohonan peminjaman ruang rapat",
	"Laporan realisasi fisik dan keuangan bulanan",
	"Sehubungan dengan akan diselenggarakannya kegiatan Bimbingan Teknis Pengelolaan Arsip Dinamis Tahun 2026, bersama ini kami mengundang Saudara untuk hadir pada kegiatan tersebut yang akan dilaksanakan pada hari Senin, 10 Agustus 2026, bertempat di Aula Kantor Dinas Perpustakaan dan Kearsipan. Acara dimulai pukul 08.00 WIB sampai dengan selesai. Mengingat pentingnya kegiatan ini, kehadiran Saudara sangat diharapkan. Atas perhatian dan kerja samanya diucapkan terima kasih.",
	"Dalam rangka tertib administrasi keuangan, bersama ini disampaikan laporan realisasi anggaran belanja operasional Triwulan II Tahun Anggaran 2026 Dinas Pendidikan Kabupaten. Realisasi belanja pegawai sebesar Rp 2.500.000.000 dari pagu Rp 2.800.000.000 atau 89 persen, belanja barang dan jasa sebesar Rp 1.200.000.000 dari pagu Rp 1.500.000.000 atau 80 persen. Secara keseluruhan realisasi anggaran mencapai 85 persen dari total pagu yang ditetapkan. Laporan ini disusun sebagai bahan evaluasi kinerja dan dasar penyusunan laporan keuangan akhir tahun.",
}

func loadQueries(cfg Config) ([]string, error) {
	var queries []string
	if cfg.QueriesFile != "" {
		f, err := os.Open(cfg.QueriesFile)
		if err != nil {
			return nil, err
		}
		defer f.Close()
		sc := bufio.NewScanner(f)
		for sc.Scan() {
			line := strings.TrimSpace(sc.Text())
			if line != "" {
				queries = append(queries, line)
			}
		}
		if err := sc.Err(); err != nil {
			return nil, err
		}
	} else {
		queries = defaultQueries
	}
	if cfg.QueryCount > 0 && cfg.QueryCount < len(queries) {
		queries = queries[:cfg.QueryCount]
	}
	return queries, nil
}

// ---------- Statistik ----------

func median(xs []float64) float64 {
	s := append([]float64(nil), xs...)
	sort.Float64s(s)
	n := len(s)
	if n == 0 {
		return 0
	}
	if n%2 == 1 {
		return s[n/2]
	}
	return (s[n/2-1] + s[n/2]) / 2
}

func percentile(xs []float64, p float64) float64 {
	s := append([]float64(nil), xs...)
	sort.Float64s(s)
	if len(s) == 0 {
		return 0
	}
	idx := int(p * float64(len(s)-1))
	return s[idx]
}

func mean(xs []float64) float64 {
	if len(xs) == 0 {
		return 0
	}
	var sum float64
	for _, x := range xs {
		sum += x
	}
	return sum / float64(len(xs))
}

// ---------- Benchmark ----------

type queryResult struct {
	Query       string
	PGMedian    float64 // ms
	MeiliMedian float64 // ms
	HybMedian   float64 // ms (0 jika hybrid tidak diuji)
	Top1Same    bool
	Jaccard3    float64
	JaccardK    float64
	HybTop1Same bool
	HybJaccardK float64
}

type report struct {
	Results    []queryResult
	PGP50      float64
	PGP95      float64
	PGP99      float64
	PGMean     float64
	MeiliP50   float64
	MeiliP95   float64
	MeiliP99   float64
	MeiliMean  float64
	HybP50     float64
	HybP95     float64
	HybP99     float64
	HybMean    float64
	Top1HitPct float64
	AvgJ3      float64
	AvgJK      float64
	AvgSpeedup float64
	HybTop1Pct float64
	AvgHybJK   float64
}

func jaccard(a, b []Hit) float64 {
	setA := map[string]bool{}
	for _, h := range a {
		setA[h.Kode] = true
	}
	inter, union := 0, len(setA)
	for _, h := range b {
		if setA[h.Kode] {
			inter++
		} else {
			union++
		}
	}
	if union == 0 {
		return 0
	}
	return float64(inter) / float64(union)
}

func meiliToHits(h []meiliHit) []Hit {
	out := make([]Hit, 0, len(h))
	for _, mh := range h {
		out = append(out, Hit{
			ID: mh.ID, Kode: mh.Kode, Deskripsi: mh.Deskripsi, Path: mh.Path,
			Similarity: mh.RankingScore, // _rankingScore Meilisearch (skala berbeda dgn cosine)
		})
	}
	return out
}

func runBenchmark(cfg Config, pg *PG, meili *MeiliClient, queries []string) (*report, error) {
	rep := &report{}

	// Kumpulkan median latensi per engine untuk agregasi
	var pgMeds, meiliMeds, hybMeds []float64

	// measure: jalankan fn `runs` kali, kembalikan median latensi + hits run terakhir
	measure := func(runs int, fn func() ([]Hit, error)) (float64, []Hit, error) {
		lats := make([]float64, 0, runs)
		var hits []Hit
		for i := 0; i < runs; i++ {
			t0 := time.Now()
			h, err := fn()
			lats = append(lats, float64(time.Since(t0).Microseconds())/1000.0)
			if err != nil {
				return 0, nil, err
			}
			hits = h
		}
		return median(lats), hits, nil
	}

	for qi, q := range queries {
		// 1. Embed query sekali via Gemini (sama untuk semua engine)
		vec, err := embedText(cfg.GeminiKey, q)
		if err != nil {
			fmt.Printf("⚠️  Query %d gagal embed: %v (dilewati)\n", qi+1, err)
			continue
		}

		r := queryResult{Query: shortQuery(q)}

		// 2. Warmup (tidak dihitung): index load, mmap, koneksi
		for i := 0; i < 2; i++ {
			if _, err := pg.searchPG(vec, cfg.TopK); err != nil {
				return nil, fmt.Errorf("warmup pg query %d: %w", qi, err)
			}
			if _, err := meili.searchMeili(cfg.IndexName, cfg.Embedder, q, vec, 1.0, cfg.TopK); err != nil {
				return nil, fmt.Errorf("warmup meili query %d: %w", qi, err)
			}
			if cfg.Hybrid {
				if _, err := meili.searchMeili(cfg.IndexName, cfg.Embedder, q, vec, 0.5, cfg.TopK); err != nil {
					return nil, fmt.Errorf("warmup meili hybrid query %d: %w", qi, err)
				}
			}
		}

		var pgHits, meiliHits []Hit

		// 3. Ukur dengan urutan bergantian (hindari bias urutan): genap pg dulu, ganjil meili dulu
		if qi%2 == 0 {
			r.PGMedian, pgHits, err = measure(cfg.Runs, func() ([]Hit, error) { return pg.searchPG(vec, cfg.TopK) })
			if err != nil {
				return nil, fmt.Errorf("query %d pgvector: %w", qi, err)
			}
			r.MeiliMedian, meiliHits, err = measure(cfg.Runs, func() ([]Hit, error) {
				h, e := meili.searchMeili(cfg.IndexName, cfg.Embedder, q, vec, 1.0, cfg.TopK)
				return meiliToHits(h), e
			})
			if err != nil {
				return nil, fmt.Errorf("query %d meili: %w", qi, err)
			}
		} else {
			r.MeiliMedian, meiliHits, err = measure(cfg.Runs, func() ([]Hit, error) {
				h, e := meili.searchMeili(cfg.IndexName, cfg.Embedder, q, vec, 1.0, cfg.TopK)
				return meiliToHits(h), e
			})
			if err != nil {
				return nil, fmt.Errorf("query %d meili: %w", qi, err)
			}
			r.PGMedian, pgHits, err = measure(cfg.Runs, func() ([]Hit, error) { return pg.searchPG(vec, cfg.TopK) })
			if err != nil {
				return nil, fmt.Errorf("query %d pgvector: %w", qi, err)
			}
		}
		pgMeds = append(pgMeds, r.PGMedian)
		meiliMeds = append(meiliMeds, r.MeiliMedian)

		// 4. Kualitas: top-1 sama? Jaccard top-3 & top-K
		if len(pgHits) > 0 && len(meiliHits) > 0 {
			r.Top1Same = pgHits[0].Kode == meiliHits[0].Kode
			r.Jaccard3 = jaccard(pgHits[:min(3, len(pgHits))], meiliHits[:min(3, len(meiliHits))])
			r.JaccardK = jaccard(pgHits, meiliHits)
		}

		// 5. Hybrid 50/50 (opsional)
		if cfg.Hybrid {
			var hybHits []Hit
			r.HybMedian, hybHits, err = measure(cfg.Runs, func() ([]Hit, error) {
				h, e := meili.searchMeili(cfg.IndexName, cfg.Embedder, q, vec, 0.5, cfg.TopK)
				return meiliToHits(h), e
			})
			if err != nil {
				return nil, fmt.Errorf("query %d meili hybrid: %w", qi, err)
			}
			hybMeds = append(hybMeds, r.HybMedian)
			if len(pgHits) > 0 && len(hybHits) > 0 {
				r.HybTop1Same = pgHits[0].Kode == hybHits[0].Kode
				r.HybJaccardK = jaccard(pgHits, hybHits)
			}
		}

		rep.Results = append(rep.Results, r)

		// Progress
		speedup := 0.0
		if r.MeiliMedian > 0 {
			speedup = r.PGMedian / r.MeiliMedian
		}
		marker := "✓"
		if !r.Top1Same {
			marker = "△"
		}
		fmt.Printf("\r📈 Query %d/%d %s", qi+1, len(queries), marker)
		if cfg.Verbose {
			fmt.Printf("\n   pg=%.2fms meili=%.2fms (%.2fx) top1=%v J3=%.2f JK=%.2f",
				r.PGMedian, r.MeiliMedian, speedup, r.Top1Same, r.Jaccard3, r.JaccardK)
		}
	}
	fmt.Println()

	if len(pgMeds) == 0 {
		return nil, fmt.Errorf("tidak ada query yang berhasil dijalankan")
	}

	// Agregasi
	rep.PGP50, rep.PGP95, rep.PGP99, rep.PGMean = percentile(pgMeds, 0.5), percentile(pgMeds, 0.95), percentile(pgMeds, 0.99), mean(pgMeds)
	rep.MeiliP50, rep.MeiliP95, rep.MeiliP99, rep.MeiliMean = percentile(meiliMeds, 0.5), percentile(meiliMeds, 0.95), percentile(meiliMeds, 0.99), mean(meiliMeds)
	if len(hybMeds) > 0 {
		rep.HybP50, rep.HybP95, rep.HybP99, rep.HybMean = percentile(hybMeds, 0.5), percentile(hybMeds, 0.95), percentile(hybMeds, 0.99), mean(hybMeds)
	}
	var top1ok, j3sum, jksum, spsum, hybTop1ok, hybJsum float64
	n := float64(len(rep.Results))
	for _, r := range rep.Results {
		if r.Top1Same {
			top1ok++
		}
		j3sum += r.Jaccard3
		jksum += r.JaccardK
		if r.MeiliMedian > 0 {
			spsum += r.PGMedian / r.MeiliMedian
		}
		if r.HybMedian > 0 {
			if r.HybTop1Same {
				hybTop1ok++
			}
			hybJsum += r.HybJaccardK
		}
	}
	rep.Top1HitPct = top1ok / n * 100
	rep.AvgJ3 = j3sum / n
	rep.AvgJK = jksum / n
	rep.AvgSpeedup = spsum / n
	if len(hybMeds) > 0 {
		rep.HybTop1Pct = hybTop1ok / n * 100
		rep.AvgHybJK = hybJsum / n
	}

	return rep, nil
}

func shortQuery(q string) string {
	if len(q) <= 44 {
		return q
	}
	return q[:41] + "..."
}

// ---------- Laporan ----------

func printReport(cfg Config, rep *report) {
	fmt.Println()
	fmt.Println("==================================================================")
	fmt.Println("  HASIL BENCHMARK  (median latensi per query, top-" + fmt.Sprint(cfg.TopK) + ")")
	fmt.Println("==================================================================")
	fmt.Printf("%-44s %8s %9s %8s %6s %6s %6s\n", "QUERY", "pgv(ms)", "meili(ms)", "x-faster", "top1=", "J3", "J10")
	fmt.Println("------------------------------------------------------------------")
	for _, r := range rep.Results {
		speedup := 0.0
		if r.MeiliMedian > 0 {
			speedup = r.PGMedian / r.MeiliMedian
		}
		mark := "✓"
		if !r.Top1Same {
			mark = "✗"
		}
		fmt.Printf("%-44s %8.2f %9.2f %8.2f %6s %6.2f %6.2f\n",
			r.Query, r.PGMedian, r.MeiliMedian, speedup, mark, r.Jaccard3, r.JaccardK)
	}
	fmt.Println("------------------------------------------------------------------")

	col := func(label string, p50, p95, p99, m float64) {
		fmt.Printf("  %-14s p50=%8.2fms  p95=%8.2fms  p99=%8.2fms  mean=%8.2fms\n", label, p50, p95, p99, m)
	}
	fmt.Println("  AGREGASI (median antar query):")
	col("pgvector", rep.PGP50, rep.PGP95, rep.PGP99, rep.PGMean)
	col("meili vector", rep.MeiliP50, rep.MeiliP95, rep.MeiliP99, rep.MeiliMean)
	if rep.HybP50 > 0 {
		col("meili hybrid", rep.HybP50, rep.HybP95, rep.HybP99, rep.HybMean)
	}
	fmt.Printf("\n  Rata-rata percepatan meili vs pgvector : %.2fx\n", rep.AvgSpeedup)
	fmt.Printf("  Top-1 sama dengan pgvector             : %.1f%%\n", rep.Top1HitPct)
	fmt.Printf("  Rata-rata Jaccard top-3 vs pgvector    : %.2f\n", rep.AvgJ3)
	fmt.Printf("  Rata-rata Jaccard top-%d vs pgvector   : %.2f\n", cfg.TopK, rep.AvgJK)
	if rep.HybP50 > 0 {
		fmt.Printf("  Hybrid top-1 sama dengan pgvector      : %.1f%%\n", rep.HybTop1Pct)
		fmt.Printf("  Rata-rata Jaccard hybrid vs pgvector   : %.2f\n", rep.AvgHybJK)
	}
	fmt.Println("==================================================================")
	fmt.Println("  Catatan metodologi:")
	fmt.Println("  - Latensi = round-trip end-to-end (HTTP/DB call dari proses benchmark).")
	fmt.Println("  - pgvector: exact kNN (seq scan, tanpa index HNSW). Meilisearch: HNSW.")
	fmt.Println("  - Dataset kecil (5.534 dokumen) → selisih didominasi overhead tetap")
	fmt.Println("    (HTTP + serialisasi JSON vektor 768-dims), bukan kecepatan komputasi.")
	fmt.Println("==================================================================")
}
