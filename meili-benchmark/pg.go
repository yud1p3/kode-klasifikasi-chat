package main

import (
	"context"
	"database/sql"
	"fmt"
	"strconv"
	"strings"
	"time"

	_ "github.com/jackc/pgx/v5/stdlib"
)

type PG struct {
	db *sql.DB
}

func newPG(url string) (*PG, error) {
	db, err := sql.Open("pgx", url)
	if err != nil {
		return nil, err
	}
	if err := db.Ping(); err != nil {
		return nil, err
	}
	db.SetMaxOpenConns(5)
	return &PG{db: db}, nil
}

func (p *PG) Close() { p.db.Close() }

type KlasifikasiRow struct {
	ID        int
	Kode      string
	Deskripsi string
	Path      string
	Embedding []float32
}

func parseVector(s string) ([]float32, error) {
	s = strings.TrimSpace(s)
	if len(s) < 2 || s[0] != '[' || s[len(s)-1] != ']' {
		return nil, fmt.Errorf("format vector tidak valid (prefix %s)", s[:min(len(s), 20)])
	}
	inner := s[1 : len(s)-1]
	parts := strings.Split(inner, ",")
	vec := make([]float32, 0, len(parts))
	for _, p := range parts {
		f, err := strconv.ParseFloat(strings.TrimSpace(p), 32)
		if err != nil {
			return nil, err
		}
		vec = append(vec, float32(f))
	}
	return vec, nil
}

// readAllDocs membaca semua baris klasifikasi + embedding (embedding::text untuk parse).
func (p *PG) readAllDocs() ([]KlasifikasiRow, error) {
	rows, err := p.db.QueryContext(context.Background(),
		`SELECT id, kode, deskripsi, path, embedding::text
		 FROM klasifikasi_embedding
		 WHERE embedding IS NOT NULL
		 ORDER BY id`)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var out []KlasifikasiRow
	for rows.Next() {
		var r KlasifikasiRow
		var embText string
		if err := rows.Scan(&r.ID, &r.Kode, &r.Deskripsi, &r.Path, &embText); err != nil {
			return nil, err
		}
		vec, err := parseVector(embText)
		if err != nil {
			return nil, fmt.Errorf("id %d: %w", r.ID, err)
		}
		r.Embedding = vec
		out = append(out, r)
	}
	return out, rows.Err()
}

// vectorToStr menyusun literal "[0.1,0.2,...]" untuk param ::vector.
func vectorToStr(v []float32) string {
	var sb strings.Builder
	sb.WriteByte('[')
	for i, f := range v {
		if i > 0 {
			sb.WriteByte(',')
		}
		sb.WriteString(strconv.FormatFloat(float64(f), 'f', -1, 32))
	}
	sb.WriteByte(']')
	return sb.String()
}

type Hit struct {
	ID         int
	Kode       string
	Deskripsi  string
	Path       string
	Similarity float64
}

// searchPG melakukan kNN cosine persis seperti search.rs backend (ORDER BY <=> LIMIT).
func (p *PG) searchPG(vec []float32, limit int) ([]Hit, error) {
	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()
	vecStr := vectorToStr(vec)

	rows, err := p.db.QueryContext(ctx, `
		SELECT id, kode, deskripsi, path,
		       1.0 - (embedding <=> $1::vector) AS similarity
		FROM klasifikasi_embedding
		WHERE embedding IS NOT NULL
		ORDER BY embedding <=> $1::vector
		LIMIT $2`, vecStr, limit)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var out []Hit
	for rows.Next() {
		var h Hit
		if err := rows.Scan(&h.ID, &h.Kode, &h.Deskripsi, &h.Path, &h.Similarity); err != nil {
			return nil, err
		}
		out = append(out, h)
	}
	return out, rows.Err()
}

func (p *PG) totalSize(table string) (string, error) {
	var s string
	err := p.db.QueryRow(
		`SELECT pg_size_pretty(pg_total_relation_size($1))`, table).Scan(&s)
	return s, err
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}
