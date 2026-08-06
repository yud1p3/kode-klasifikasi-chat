package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

const (
	geminiEmbedURL = "https://generativelanguage.googleapis.com/v1beta/models/gemini-embedding-2:embedContent"
	embedDims      = 768
)

var geminiHTTP = &http.Client{Timeout: 60 * time.Second}

// embedText memanggil Gemini embedding (sama persis dengan backend: model gemini-embedding-2, 768 dims).
func embedText(apiKey, text string) ([]float32, error) {
	body := map[string]any{
		"content":              map[string]any{"parts": []map[string]any{{"text": text}}},
		"outputDimensionality": embedDims,
	}
	payload, _ := json.Marshal(body)

	// Retry sederhana saat rate limit (429), hingga 6 kali
	for attempt := 1; attempt <= 6; attempt++ {
		url := fmt.Sprintf("%s?key=%s", geminiEmbedURL, apiKey)
		req, err := http.NewRequest("POST", url, bytes.NewReader(payload))
		if err != nil {
			return nil, err
		}
		req.Header.Set("Content-Type", "application/json")
		resp, err := geminiHTTP.Do(req)
		if err != nil {
			return nil, err
		}
		data, _ := io.ReadAll(resp.Body)
		resp.Body.Close()

		if resp.StatusCode == 429 || resp.StatusCode == 503 {
			time.Sleep(2 * time.Duration(attempt) * time.Second)
			continue
		}
		if resp.StatusCode != 200 {
			return nil, fmt.Errorf("Gemini embed %d: %s", resp.StatusCode, string(data[:min(len(data), 200)]))
		}
		var out struct {
			Embedding struct {
				Values []float32 `json:"values"`
			} `json:"embedding"`
		}
		if err := json.Unmarshal(data, &out); err != nil {
			return nil, err
		}
		if len(out.Embedding.Values) == 0 {
			return nil, fmt.Errorf("embedding kosong")
		}
		return out.Embedding.Values, nil
	}
	return nil, fmt.Errorf("Gemini rate limit terus-menerus (429)")
}
