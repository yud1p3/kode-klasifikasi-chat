package main

import (
	"fmt"
	"time"
)

// indexDocuments: (re)buat index Meilisearch, set settings (searchable + embedder),
// lalu push semua dokumen dari PG dalam batch dengan field _vectors.
func indexDocuments(cfg Config, pg *PG, meili *MeiliClient) error {
	docs, err := pg.readAllDocs()
	if err != nil {
		return fmt.Errorf("baca dokumen dari PG: %w", err)
	}
	fmt.Printf("📄 Membaca %d dokumen + embedding dari PostgreSQL\n", len(docs))
	if len(docs) == 0 {
		return fmt.Errorf("tidak ada data di tabel klasifikasi_embedding")
	}

	if cfg.Force {
		fmt.Printf("🔄 Force reindex: hapus index '%s'...\n", cfg.IndexName)
		if err := meili.deleteIndex(cfg.IndexName); err != nil {
			return err
		}
		if err := meili.createIndex(cfg.IndexName, "id"); err != nil {
			return err
		}
		fmt.Println("✅ Index dibuat")

		settings := map[string]any{
			"searchableAttributes": []string{"kode", "deskripsi", "path"},
			"embedders": map[string]any{
				cfg.Embedder: map[string]any{"dimensions": len(docs[0].Embedding)},
			},
		}
		if err := meili.updateSettings(cfg.IndexName, settings); err != nil {
			return fmt.Errorf("set settings: %w", err)
		}
		fmt.Printf("✅ Settings: searchable (kode, deskripsi, path) + embedder '%s' (%d dims)\n", cfg.Embedder, len(docs[0].Embedding))
	}

	// Push dokumen dalam batch
	const batchSize = 500
	start := time.Now()
	total := 0
	for i := 0; i < len(docs); i += batchSize {
		end := min(i+batchSize, len(docs))
		batch := make([]map[string]any, 0, end-i)
		for _, d := range docs[i:end] {
			batch = append(batch, map[string]any{
				"id":        d.ID,
				"kode":      d.Kode,
				"deskripsi": d.Deskripsi,
				"path":      d.Path,
				"_vectors": map[string]any{
					cfg.Embedder: d.Embedding,
				},
			})
		}
		if err := meili.addDocuments(cfg.IndexName, batch); err != nil {
			return fmt.Errorf("batch %d: %w", i/batchSize+1, err)
		}
		total += len(batch)
		fmt.Printf("\r📊 Indexing: %d/%d dokumen (%.0f%%)", total, len(docs), float64(total)/float64(len(docs))*100)
	}
	fmt.Printf("\n✅ Selesai indexing %d dokumen dalam %s\n", total, time.Since(start).Round(time.Millisecond))
	return nil
}
