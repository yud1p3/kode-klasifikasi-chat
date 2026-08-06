package main

import (
	"flag"
	"fmt"
	"os"
	"strings"
)

// ---------- Config ----------

type Config struct {
	DBURL       string
	GeminiKey   string
	MeiliHost   string
	MeiliKey    string
	IndexName   string
	Embedder    string
	Force       bool
	SkipIndex   bool
	QueriesFile string
	QueryCount  int
	TopK        int
	Runs        int
	Hybrid      bool
	Verbose     bool
}

func parseFlags() Config {
	var cfg Config
	flag.StringVar(&cfg.DBURL, "db-url", envOr("DATABASE_URL", "postgres://postgres:postgres@localhost:5432/klasifikasi_arsip"), "PostgreSQL URL")
	flag.StringVar(&cfg.GeminiKey, "gemini-key", "", "Gemini API key untuk embedding query benchmark (default: env GEMINI_API_KEY / GEMINI_API_KEYS)")
	flag.StringVar(&cfg.MeiliHost, "meili-host", envOr("MEILI_HOST", "http://localhost:7700"), "Meilisearch host")
	flag.StringVar(&cfg.MeiliKey, "meili-key", envOr("MEILI_MASTER_KEY", ""), "Meilisearch master key")
	flag.StringVar(&cfg.IndexName, "index", "klasifikasi_embedding", "Nama index Meilisearch")
	flag.StringVar(&cfg.Embedder, "embedder", "userProvided", "Nama embedder di settings Meilisearch")
	flag.BoolVar(&cfg.Force, "force", false, "Hapus & buat ulang index (reindex penuh)")
	flag.BoolVar(&cfg.SkipIndex, "skip-index", false, "Lewati indexing, langsung benchmark")
	flag.StringVar(&cfg.QueriesFile, "queries", "", "File berisi query benchmark (satu per baris). Default: query bawaan")
	flag.IntVar(&cfg.QueryCount, "q", 0, "Batasi jumlah query (0 = semua)")
	flag.IntVar(&cfg.TopK, "topk", 10, "Jumlah hasil top-k")
	flag.IntVar(&cfg.Runs, "runs", 10, "Iterasi per query per engine")
	flag.BoolVar(&cfg.Hybrid, "hybrid", false, "Juga uji hybrid search Meilisearch (semanticRatio 0.5)")
	flag.BoolVar(&cfg.Verbose, "verbose", false, "Tampilkan rincian per query")
	help := flag.Bool("help", false, "Tampilkan bantuan")
	flag.Parse()

	if *help {
		fmt.Println("Meili Benchmark — Bandingkan pgvector vs Meilisearch (vector & hybrid search)")
		fmt.Println()
		fmt.Println("Penggunaan:")
		fmt.Println("  ./meili-benchmark -meili-key KEY -force        # index ulang + benchmark")
		fmt.Println("  ./meili-benchmark -meili-key KEY -skip-index   # benchmark saja")
		fmt.Println()
		fmt.Println("Flags:")
		flag.PrintDefaults()
		os.Exit(0)
	}

	if cfg.GeminiKey == "" {
		cfg.GeminiKey = envOr("GEMINI_API_KEYS", "")
		if cfg.GeminiKey == "" {
			cfg.GeminiKey = os.Getenv("GEMINI_API_KEY")
		} else {
			cfg.GeminiKey = strings.Split(cfg.GeminiKey, ",")[0] // multi-key → pakai key pertama
		}
	}
	if cfg.MeiliKey == "" {
		fmt.Println("❌ Meilisearch master key tidak ditemukan. Set -meili-key atau env MEILI_MASTER_KEY.")
		os.Exit(1)
	}
	if cfg.GeminiKey == "" {
		fmt.Println("❌ Gemini API key tidak ditemukan. Set -gemini-key atau env GEMINI_API_KEY / GEMINI_API_KEYS.")
		os.Exit(1)
	}
	return cfg
}

func envOr(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}

func mask(s string) string {
	if len(s) <= 8 {
		return "***"
	}
	return s[:4] + "..." + s[len(s)-4:]
}

// ---------- Main ----------

func main() {
	cfg := parseFlags()

	fmt.Println("⚡ Meili Benchmark — pgvector vs Meilisearch")
	fmt.Println("============================================")
	fmt.Printf("🔑 Gemini key : %s\n", mask(cfg.GeminiKey))
	fmt.Printf("🗄️  Meilisearch: %s (index '%s', embedder '%s')\n", cfg.MeiliHost, cfg.IndexName, cfg.Embedder)

	pg, err := newPG(cfg.DBURL)
	if err != nil {
		fmt.Printf("❌ Gagal koneksi PostgreSQL: %v\n", err)
		os.Exit(1)
	}
	defer pg.Close()
	fmt.Println("✅ Koneksi PostgreSQL berhasil")

	meili := newMeili(cfg.MeiliHost, cfg.MeiliKey)
	if err := meili.ping(); err != nil {
		fmt.Printf("❌ Gagal koneksi Meilisearch: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("✅ Koneksi Meilisearch berhasil (%s)\n", mask(cfg.MeiliKey))

	// ---------- 1. Indexing ----------
	if !cfg.SkipIndex {
		if err := indexDocuments(cfg, pg, meili); err != nil {
			fmt.Printf("❌ Indexing gagal: %v\n", err)
			os.Exit(1)
		}
	}

	stats, err := meili.indexStats(cfg.IndexName)
	if err != nil {
		fmt.Printf("⚠️  Gagal baca stats index: %v\n", err)
	} else {
		fmt.Printf("📊 Index '%s': %d dokumen, %d MB (vector DB: %d MB)\n",
			cfg.IndexName, stats.NumberOfDocuments, stats.IndexSize/1024/1024, stats.VectorDB.TotalVectorSize/1024/1024)
	}

	// ---------- 2. Benchmark ----------
	queries, err := loadQueries(cfg)
	if err != nil {
		fmt.Printf("❌ Gagal memuat query: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("🎯 Benchmark: %d query × %d iterasi, top-%d%s\n", len(queries), cfg.Runs, cfg.TopK, map[bool]string{true: ", + hybrid 50/50", false: ""}[cfg.Hybrid])
	fmt.Println()

	report, err := runBenchmark(cfg, pg, meili, queries)
	if err != nil {
		fmt.Printf("❌ Benchmark gagal: %v\n", err)
		os.Exit(1)
	}
	printReport(cfg, report)

	// ---------- 3. Ukuran data ----------
	fmt.Println()
	fmt.Println("📦 Perbandingan Ukuran Data & Memori")
	fmt.Println("------------------------------------")
	pgSize, err := pg.totalSize("klasifikasi_embedding")
	if err != nil {
		fmt.Printf("PG total size: %v\n", err)
	} else {
		fmt.Printf("PostgreSQL (table+indexes): %s\n", pgSize)
	}
	meiliStats, _ := meili.globalStats()
	if meiliStats != nil {
		fmt.Printf("Meilisearch seluruh DB      : %d MB (semua index)\n", meiliStats.DatabaseSize/1024/1024)
	}
}
