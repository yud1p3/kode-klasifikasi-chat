package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"time"
)

// ---------- Meilisearch REST client (tanpa SDK, API stabil) ----------

type MeiliClient struct {
	host string
	key  string
	http *http.Client
}

func newMeili(host, key string) *MeiliClient {
	return &MeiliClient{host: host, key: key, http: &http.Client{Timeout: 120 * time.Second}}
}

func (m *MeiliClient) do(method, path string, body any, out any) error {
	var rdr io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return err
		}
		rdr = bytes.NewReader(b)
	}
	req, err := http.NewRequest(method, m.host+path, rdr)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+m.key)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := m.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}
	if resp.StatusCode >= 400 {
		var e struct {
			Message string `json:"message"`
			Code    string `json:"code"`
		}
		_ = json.Unmarshal(data, &e)
		return fmt.Errorf("Meilisearch %s %s (%d): %s", method, path, resp.StatusCode, e.Message)
	}
	if out != nil && len(data) > 0 {
		return json.Unmarshal(data, out)
	}
	return nil
}

func (m *MeiliClient) ping() error {
	var out struct {
		Status string `json:"status"`
	}
	return m.do("GET", "/health", nil, &out)
}

// taskResult adalah struktur minimal respon task Meilisearch.
// Endpoint modern mengembalikan taskUid (bukan uid), jadi dua-duanya diparse.
type taskResult struct {
	UID     int    `json:"uid"`
	TaskUID int    `json:"taskUid"`
	Status  string `json:"status"`
	Error   *struct {
		Message string `json:"message"`
		Code    string `json:"code"`
	} `json:"error"`
}

func (t *taskResult) ID() int {
	if t.TaskUID != 0 {
		return t.TaskUID
	}
	return t.UID
}

func (m *MeiliClient) waitTask(uid int, timeout time.Duration) error {
	deadline := time.Now().Add(timeout)
	for {
		var t taskResult
		if err := m.do("GET", fmt.Sprintf("/tasks/%d", uid), nil, &t); err != nil {
			return err
		}
		switch t.Status {
		case "succeeded":
			return nil
		case "failed":
			if t.Error != nil {
				return fmt.Errorf("task %d failed: %s (%s)", uid, t.Error.Message, t.Error.Code)
			}
			return fmt.Errorf("task %d failed", uid)
		case "canceled":
			return fmt.Errorf("task %d canceled", uid)
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("timeout menunggu task %d (status: %s)", uid, t.Status)
		}
		time.Sleep(100 * time.Millisecond)
	}
}

func (m *MeiliClient) deleteIndex(uid string) error {
	// DELETE index; error 404 dianggap sukses (index memang belum ada)
	req, err := http.NewRequest("DELETE", m.host+"/indexes/"+uid, nil)
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+m.key)
	resp, err := m.http.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	data, _ := io.ReadAll(resp.Body)
	if resp.StatusCode == 404 {
		return nil
	}
	if resp.StatusCode >= 400 {
		var e struct {
			Message string `json:"message"`
		}
		_ = json.Unmarshal(data, &e)
		return fmt.Errorf("DELETE /indexes/%s -> %d: %s", uid, resp.StatusCode, e.Message)
	}
	// Respon 202 berisi task; tunggu sampai index benar-benar terhapus
	var t taskResult
	_ = json.Unmarshal(data, &t)
	if t.ID() > 0 {
		return m.waitTask(t.ID(), 60*time.Second)
	}
	return nil
}

func (m *MeiliClient) createIndex(uid, primaryKey string) error {
	var t taskResult
	if err := m.do("POST", "/indexes", map[string]any{"uid": uid, "primaryKey": primaryKey}, &t); err != nil {
		return err
	}
	return m.waitTask(t.ID(), 60*time.Second)
}

func (m *MeiliClient) updateSettings(uid string, settings map[string]any) error {
	var t taskResult
	if err := m.do("PATCH", "/indexes/"+uid+"/settings", settings, &t); err != nil {
		return err
	}
	return m.waitTask(t.ID(), 60*time.Second)
}

func (m *MeiliClient) addDocuments(uid string, docs []map[string]any) error {
	var t taskResult
	if err := m.do("POST", "/indexes/"+uid+"/documents", docs, &t); err != nil {
		return err
	}
	return m.waitTask(t.ID(), 120*time.Second)
}

// ---------- Search ----------

type meiliSearchReq struct {
	Q      string     `json:"q,omitempty"`
	Vector []float32  `json:"vector,omitempty"`
	Hybrid *hybridReq `json:"hybrid,omitempty"`
	Limit  int        `json:"limit"`
}

type hybridReq struct {
	Embedder      string  `json:"embedder"`
	SemanticRatio float64 `json:"semanticRatio"`
}

type meiliHit struct {
	ID           int     `json:"id"`
	Kode         string  `json:"kode"`
	Deskripsi    string  `json:"deskripsi"`
	Path         string  `json:"path"`
	RankingScore float64 `json:"_rankingScore"`
}

type meiliSearchResp struct {
	Hits []meiliHit `json:"hits"`
}

// searchMeili: semanticRatio 1.0 = murni vector (setara pgvector), 0.5 = hybrid keyword+semantic.
func (m *MeiliClient) searchMeili(uid, embedder, q string, vector []float32, semanticRatio float64, limit int) ([]meiliHit, error) {
	req := meiliSearchReq{
		Vector: vector,
		Hybrid: &hybridReq{Embedder: embedder, SemanticRatio: semanticRatio},
		Limit:  limit,
	}
	if semanticRatio < 1.0 {
		req.Q = q
	}
	var resp meiliSearchResp
	if err := m.do("POST", "/indexes/"+uid+"/search", req, &resp); err != nil {
		return nil, err
	}
	return resp.Hits, nil
}

// ---------- Stats ----------

type indexStatsResp struct {
	NumberOfDocuments int64 `json:"numberOfDocuments"`
	IsIndexing        bool  `json:"isIndexing"`
	IndexSize         int64 `json:"indexSize"`
	VectorDB          struct {
		NumberOfEmbeddings int64 `json:"numberOfEmbeddings"`
		TotalVectorSize    int64 `json:"totalVectorSize"`
	} `json:"vectorDB"`
}

func (m *MeiliClient) indexStats(uid string) (*indexStatsResp, error) {
	var s indexStatsResp
	if err := m.do("GET", "/indexes/"+uid+"/stats", nil, &s); err != nil {
		return nil, err
	}
	return &s, nil
}

type globalStatsResp struct {
	DatabaseSize int64 `json:"databaseSize"`
}

func (m *MeiliClient) globalStats() (*globalStatsResp, error) {
	var s globalStatsResp
	if err := m.do("GET", "/stats", nil, &s); err != nil {
		return nil, err
	}
	return &s, nil
}
