package main

import (
	"context"
	"encoding/json"
	"errors"
	"flag"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/gotd/td/telegram"
	"github.com/gotd/td/telegram/peers"
	"github.com/gotd/td/telegram/query"
	"github.com/gotd/td/tg"
	tdlstorage "github.com/iyear/tdl/core/storage"
	tdlcoreclient "github.com/iyear/tdl/core/tclient"
	"github.com/iyear/tdl/core/tmedia"
	"github.com/iyear/tdl/core/util/tutil"
	"github.com/iyear/tdl/pkg/kv"
	tdlclient "github.com/iyear/tdl/pkg/tclient"
)

const (
	defaultNamespace = "default"
	requestTimeout   = 90 * time.Second
)

type response struct {
	OK     bool        `json:"ok"`
	Result interface{} `json:"result,omitempty"`
	Error  string      `json:"error,omitempty"`
}

type globalOptions struct {
	storagePath string
	namespace   string
}

type statusResult struct {
	Configured  bool    `json:"configured"`
	Initialized bool    `json:"initialized"`
	Authorized  bool    `json:"authorized"`
	State       string  `json:"state"`
	Message     string  `json:"message"`
	QRLink      *string `json:"qrLink"`
	Username    *string `json:"username"`
	DisplayName *string `json:"displayName"`
}

type chatResult struct {
	ID              int64   `json:"id"`
	Title           string  `json:"title"`
	ChatType        string  `json:"chatType"`
	UnreadCount     int     `json:"unreadCount"`
	LastMessageID   *int64  `json:"lastMessageId"`
	LastMessageText *string `json:"lastMessageText"`
}

type messageResult struct {
	ID          int64   `json:"id"`
	Date        *string `json:"date"`
	Text        *string `json:"text"`
	MediaKind   string  `json:"mediaKind"`
	MediaType   *string `json:"mediaType"`
	MimeType    *string `json:"mimeType"`
	FileName    *string `json:"fileName"`
	FileSize    *int64  `json:"fileSize"`
	Width       *int64  `json:"width"`
	Height      *int64  `json:"height"`
	Duration    *int64  `json:"duration"`
	Previewable bool    `json:"previewable"`
}

func main() {
	if err := run(); err != nil {
		write(response{OK: false, Error: err.Error()})
		os.Exit(1)
	}
}

func run() error {
	if len(os.Args) < 2 {
		return errors.New("missing command")
	}

	cmd := os.Args[1]
	switch cmd {
	case "status":
		opts, err := parseFlags(cmd, os.Args[2:])
		if err != nil {
			return err
		}
		result, err := status(opts)
		if err != nil {
			return err
		}
		write(response{OK: true, Result: result})
	case "chats":
		fs, opts := newFlagSet(cmd)
		limit := fs.Int("limit", 100, "max dialogs")
		if err := fs.Parse(os.Args[2:]); err != nil {
			return err
		}
		result, err := chats(opts, clamp(*limit, 1, 500))
		if err != nil {
			return err
		}
		write(response{OK: true, Result: result})
	case "messages":
		fs, opts := newFlagSet(cmd)
		chatID := fs.String("chat-id", "", "chat id or username")
		limit := fs.Int("limit", 50, "max messages")
		if err := fs.Parse(os.Args[2:]); err != nil {
			return err
		}
		if strings.TrimSpace(*chatID) == "" {
			return errors.New("missing --chat-id")
		}
		result, err := messages(opts, strings.TrimSpace(*chatID), clamp(*limit, 1, 500))
		if err != nil {
			return err
		}
		write(response{OK: true, Result: result})
	default:
		return fmt.Errorf("unknown command: %s", cmd)
	}

	return nil
}

func parseFlags(name string, args []string) (globalOptions, error) {
	fs, opts := newFlagSet(name)
	if err := fs.Parse(args); err != nil {
		return globalOptions{}, err
	}
	return opts, nil
}

func newFlagSet(name string) (*flag.FlagSet, globalOptions) {
	opts := globalOptions{
		storagePath: defaultStoragePath(),
		namespace:   defaultNamespace,
	}
	fs := flag.NewFlagSet(name, flag.ContinueOnError)
	fs.SetOutput(io.Discard)
	fs.StringVar(&opts.storagePath, "storage", opts.storagePath, "tdl bolt storage path")
	fs.StringVar(&opts.namespace, "namespace", opts.namespace, "tdl namespace")
	return fs, opts
}

func status(opts globalOptions) (*statusResult, error) {
	if !fileExists(filepath.Join(opts.storagePath, opts.namespace)) {
		return &statusResult{
			Configured:  true,
			Initialized: false,
			Authorized:  false,
			State:       "missingSession",
			Message:     "未找到 tdl 登录态，请先使用 tdl 登录。",
		}, nil
	}

	ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
	defer cancel()

	var result statusResult
	err := withClient(ctx, opts, false, func(ctx context.Context, client *telegram.Client, _ tdlstorage.Storage) error {
		return client.Run(ctx, func(ctx context.Context) error {
			authStatus, err := client.Auth().Status(ctx)
			if err != nil {
				return err
			}
			result.Configured = true
			result.Initialized = true
			result.Authorized = authStatus.Authorized
			if !authStatus.Authorized {
				result.State = "loggedOut"
				result.Message = "tdl 登录态未授权，请先登录。"
				return nil
			}

			self, err := client.Self(ctx)
			if err == nil {
				result.Username = stringPtr(self.Username)
				result.DisplayName = stringPtr(visibleName(self.FirstName, self.LastName))
			}
			result.State = "authorized"
			result.Message = "已复用 tdl 登录态。"
			return nil
		})
	})
	if err != nil {
		return nil, err
	}
	return &result, nil
}

func chats(opts globalOptions, limit int) ([]chatResult, error) {
	ctx, cancel := context.WithTimeout(context.Background(), requestTimeout)
	defer cancel()

	var result []chatResult
	err := withClient(ctx, opts, true, func(ctx context.Context, client *telegram.Client, store tdlstorage.Storage) error {
		api := client.API()
		manager := peers.Options{Storage: tdlstorage.NewPeers(store)}.Build(api)
		iter := query.GetDialogs(api).BatchSize(min(limit, 100)).Iter()
		for iter.Next(ctx) {
			elem := iter.Value()
			_ = applyEntities(ctx, manager, elem.Entities.Users(), elem.Entities.Chats(), elem.Entities.Channels())
			if chat, ok := dialogToChat(elem.Dialog, elem.Peer, elem.Entities, elem.Last); ok {
				result = append(result, chat)
			}
			if len(result) >= limit {
				break
			}
		}
		return iter.Err()
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}

func messages(opts globalOptions, chatID string, limit int) ([]messageResult, error) {
	ctx, cancel := context.WithTimeout(context.Background(), requestTimeout)
	defer cancel()

	var result []messageResult
	err := withClient(ctx, opts, true, func(ctx context.Context, client *telegram.Client, store tdlstorage.Storage) error {
		manager := peers.Options{Storage: tdlstorage.NewPeers(store)}.Build(client.API())
		peer, err := tutil.GetInputPeer(ctx, manager, chatID)
		if err != nil {
			return err
		}

		iter := query.Messages(client.API()).GetHistory(peer.InputPeer()).BatchSize(min(limit, 100)).Iter()
		for iter.Next(ctx) {
			msg, ok := iter.Value().Msg.(*tg.Message)
			if !ok {
				continue
			}
			result = append(result, messageToResult(msg))
			if len(result) >= limit {
				break
			}
		}
		return iter.Err()
	})
	if err != nil {
		return nil, err
	}
	return result, nil
}

func withClient(ctx context.Context, opts globalOptions, requireAuth bool, fn func(context.Context, *telegram.Client, tdlstorage.Storage) error) (rerr error) {
	engine, err := kv.NewWithMap(map[string]string{
		"type": "bolt",
		"path": opts.storagePath,
	})
	if err != nil {
		return err
	}
	defer func() {
		if err := engine.Close(); rerr == nil && err != nil {
			rerr = err
		}
	}()

	store, err := engine.Open(opts.namespace)
	if err != nil {
		return err
	}

	client, err := tdlclient.New(ctx, tdlclient.Options{
		KV:               store,
		ReconnectTimeout: 5 * time.Minute,
	}, false)
	if err != nil {
		return err
	}

	if !requireAuth {
		return fn(ctx, client, store)
	}

	return tdlcoreclient.RunWithAuth(ctx, client, func(ctx context.Context) error {
		return fn(ctx, client, store)
	})
}

func applyEntities(ctx context.Context, manager *peers.Manager, users map[int64]*tg.User, chats map[int64]*tg.Chat, channels map[int64]*tg.Channel) error {
	userClasses := make([]tg.UserClass, 0, len(users))
	for _, user := range users {
		userClasses = append(userClasses, user)
	}
	chatClasses := make([]tg.ChatClass, 0, len(chats)+len(channels))
	for _, chat := range chats {
		chatClasses = append(chatClasses, chat)
	}
	for _, channel := range channels {
		chatClasses = append(chatClasses, channel)
	}
	return manager.Apply(ctx, userClasses, chatClasses)
}

func dialogToChat(dialog tg.DialogClass, input tg.InputPeerClass, entities peerEntities, last tg.NotEmptyMessage) (chatResult, bool) {
	id := inputPeerID(input)
	if id == 0 {
		return chatResult{}, false
	}

	chat := chatResult{
		ID:          id,
		Title:       fmt.Sprintf("%d", id),
		ChatType:    "unknown",
		UnreadCount: unreadCount(dialog),
	}

	switch peer := input.(type) {
	case *tg.InputPeerUser:
		if user, ok := entities.User(peer.UserID); ok {
			chat.Title = visibleName(user.FirstName, user.LastName)
			if chat.Title == "" {
				chat.Title = user.Username
			}
			chat.ChatType = "private"
		}
	case *tg.InputPeerChat:
		if raw, ok := entities.Chat(peer.ChatID); ok {
			chat.Title = raw.Title
			chat.ChatType = "group"
		}
	case *tg.InputPeerChannel:
		if raw, ok := entities.Channel(peer.ChannelID); ok {
			chat.Title = raw.Title
			switch {
			case raw.Broadcast:
				chat.ChatType = "channel"
			case raw.Megagroup || raw.Gigagroup:
				chat.ChatType = "group"
			default:
				chat.ChatType = "unknown"
			}
		}
	}

	if last != nil {
		msgID := int64(last.GetID())
		chat.LastMessageID = &msgID
		if msg, ok := last.(*tg.Message); ok {
			text := compact(msg.Message, 120)
			if text != "" {
				chat.LastMessageText = &text
			}
		}
	}

	if strings.TrimSpace(chat.Title) == "" {
		chat.Title = fmt.Sprintf("%d", id)
	}
	return chat, true
}

type peerEntities interface {
	User(id int64) (*tg.User, bool)
	Chat(id int64) (*tg.Chat, bool)
	Channel(id int64) (*tg.Channel, bool)
}

func messageToResult(msg *tg.Message) messageResult {
	date := time.Unix(int64(msg.Date), 0).Format(time.RFC3339)
	text := compact(msg.Message, 500)
	result := messageResult{
		ID:        int64(msg.ID),
		Date:      &date,
		Text:      stringPtr(text),
		MediaKind: "none",
	}
	if text == "" {
		result.Text = nil
	}

	media, ok := msg.GetMedia()
	if !ok {
		return result
	}

	switch typed := media.(type) {
	case *tg.MessageMediaPhoto:
		result.MediaKind = "photo"
		result.MediaType = stringPtr("photo")
		result.MimeType = stringPtr("image/jpeg")
		if info, ok := tmedia.ExtractMedia(typed); ok {
			result.FileName = stringPtr(info.Name)
			result.FileSize = int64Ptr(info.Size)
		}
		if photo, ok := typed.Photo.(*tg.Photo); ok {
			if _, width, height, ok := photoDimensions(photo); ok {
				result.Width = int64Ptr(int64(width))
				result.Height = int64Ptr(int64(height))
			}
		}
		result.Previewable = true
	case *tg.MessageMediaDocument:
		result.MediaKind = "document"
		result.MediaType = stringPtr("document")
		if doc, ok := typed.Document.(*tg.Document); ok {
			if doc.MimeType != "" {
				result.MimeType = stringPtr(strings.ToLower(doc.MimeType))
			}
			if info, ok := tmedia.ExtractMedia(typed); ok {
				result.FileName = stringPtr(info.Name)
				result.FileSize = int64Ptr(info.Size)
			} else {
				result.FileSize = int64Ptr(doc.Size)
			}
			applyDocumentAttributes(&result, doc)
		}
		result.Previewable = result.MediaKind == "photo" || result.MediaKind == "video"
	default:
		result.MediaKind = "unknown"
		result.MediaType = stringPtr(fmt.Sprintf("%T", media))
	}
	return result
}

func applyDocumentAttributes(result *messageResult, doc *tg.Document) {
	mime := strings.ToLower(doc.MimeType)
	if strings.HasPrefix(mime, "image/") {
		result.MediaKind = "photo"
		result.MediaType = stringPtr("photo")
	} else if strings.HasPrefix(mime, "video/") {
		result.MediaKind = "video"
		result.MediaType = stringPtr("video")
	} else if strings.HasPrefix(mime, "audio/") {
		result.MediaKind = "audio"
		result.MediaType = stringPtr("audio")
	}

	for _, attr := range doc.Attributes {
		switch value := attr.(type) {
		case *tg.DocumentAttributeVideo:
			result.MediaKind = "video"
			result.MediaType = stringPtr("video")
			result.Width = int64Ptr(int64(value.W))
			result.Height = int64Ptr(int64(value.H))
			result.Duration = int64Ptr(int64(value.Duration))
		case *tg.DocumentAttributeAudio:
			result.MediaKind = "audio"
			result.MediaType = stringPtr("audio")
			result.Duration = int64Ptr(int64(value.Duration))
		case *tg.DocumentAttributeImageSize:
			if result.MediaKind == "document" {
				result.MediaKind = "photo"
				result.MediaType = stringPtr("photo")
			}
			result.Width = int64Ptr(int64(value.W))
			result.Height = int64Ptr(int64(value.H))
		case *tg.DocumentAttributeFilename:
			if value.FileName != "" {
				result.FileName = stringPtr(value.FileName)
			}
		}
	}
}

func photoDimensions(photo *tg.Photo) (string, int, int, bool) {
	if len(photo.Sizes) == 0 {
		return "", 0, 0, false
	}
	for index := len(photo.Sizes) - 1; index >= 0; index-- {
		switch size := photo.Sizes[index].(type) {
		case *tg.PhotoSize:
			return size.Type, size.W, size.H, true
		case *tg.PhotoSizeProgressive:
			return size.Type, size.W, size.H, true
		}
	}
	return "", 0, 0, false
}

func unreadCount(dialog tg.DialogClass) int {
	if d, ok := dialog.(*tg.Dialog); ok {
		return d.UnreadCount
	}
	return 0
}

func inputPeerID(peer tg.InputPeerClass) int64 {
	switch value := peer.(type) {
	case *tg.InputPeerUser:
		return value.UserID
	case *tg.InputPeerChat:
		return value.ChatID
	case *tg.InputPeerChannel:
		return value.ChannelID
	default:
		return 0
	}
}

func visibleName(first, last string) string {
	first = strings.TrimSpace(first)
	last = strings.TrimSpace(last)
	switch {
	case first == "":
		return last
	case last == "":
		return first
	default:
		return first + " " + last
	}
}

func compact(value string, max int) string {
	value = strings.Join(strings.Fields(value), " ")
	if len([]rune(value)) <= max {
		return value
	}
	runes := []rune(value)
	return string(runes[:max]) + "..."
}

func defaultStoragePath() string {
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return filepath.Join(".tdl", "data")
	}
	return filepath.Join(home, ".tdl", "data")
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

func stringPtr(value string) *string {
	if strings.TrimSpace(value) == "" {
		return nil
	}
	return &value
}

func int64Ptr(value int64) *int64 {
	return &value
}

func clamp(value, minValue, maxValue int) int {
	if value < minValue {
		return minValue
	}
	if value > maxValue {
		return maxValue
	}
	return value
}

func write(value response) {
	_ = json.NewEncoder(os.Stdout).Encode(value)
}
